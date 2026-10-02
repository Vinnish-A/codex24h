#!/usr/bin/env python3
"""Mail regression tests; all delivery goes to an in-process local TLS SMTP server."""
import contextlib
from email import policy
from email.parser import BytesParser
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import queue
import smtplib
import socketserver
import sqlite3
import ssl
import subprocess
import sys
import tempfile
import threading
import time
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib
import unittest
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts/codex24h-mail'
loader = importlib.machinery.SourceFileLoader('codex24h_mail', str(SCRIPT))
spec = importlib.util.spec_from_loader(loader.name, loader)
mail = importlib.util.module_from_spec(spec)
loader.exec_module(mail)


class SMTPHandler(socketserver.StreamRequestHandler):
    def handle(self):
        self.wfile.write(b'220 localhost test SMTP\r\n')
        while line := self.rfile.readline():
            command = line.split()[0].upper()
            if command in (b'EHLO', b'HELO'):
                self.wfile.write(b'250-localhost\r\n250 SIZE 1000000\r\n')
            elif command in (b'MAIL', b'RCPT', b'RSET'):
                self.wfile.write(b'250 OK\r\n')
            elif command == b'DATA':
                time.sleep(.7)  # A slow SMTP server must not delay the notify callback.
                self.wfile.write(b'354 Send data\r\n')
                body = bytearray()
                while (part := self.rfile.readline()) != b'.\r\n':
                    if not part:
                        return
                    body.extend(part[1:] if part.startswith(b'..') else part)
                self.server.messages.put(bytes(body))
                self.wfile.write(b'250 Accepted\r\n')
            elif command == b'QUIT':
                self.wfile.write(b'221 Bye\r\n')
                return
            else:
                self.wfile.write(b'500 Unsupported\r\n')


class TLSService(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, cert, key):
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(cert, key)
        self.messages = queue.Queue()
        super().__init__(('127.0.0.1', 0), SMTPHandler)

    def get_request(self):
        conn, addr = super().get_request()
        return self.context.wrap_socket(conn, server_side=True), addr


class MailTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='codex24h-mail-test-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.home = self.base / 'codex'
        self.home.mkdir()
        self.config = self.base / 'mail.toml'
        self.state = self.base / 'state'
        self.write_config()
        with sqlite3.connect(self.home / 'state_5.sqlite') as conn:
            conn.execute('CREATE TABLE threads(id TEXT,name TEXT,title TEXT,source TEXT)')
            conn.execute('INSERT INTO threads VALUES (?,?,?,?)', ('session-1', '中文项目会话', 'not the name', 'cli'))
        with sqlite3.connect(self.home / 'goals_1.sqlite') as conn:
            conn.execute('CREATE TABLE thread_goals(thread_id TEXT,goal_id TEXT,status TEXT,updated_at_ms INTEGER)')
        self.event = {'type': 'agent-turn-complete', 'thread-id': 'session-1', 'turn-id': 'turn-1',
                      'input-messages': ['PRIVATE PROMPT'], 'last-assistant-message': 'PRIVATE ANSWER'}

    def write_config(self, **values):
        cfg = dict(enabled=True, host='localhost', security='ssl', port=465,
                   to=['recipient@example.com'], state_dir=str(self.state))
        cfg['from'] = 'sender@example.com'
        cfg.update(values)
        self.config.write_text('\n'.join(f'{k} = {json.dumps(v,ensure_ascii=False)}' for k,v in cfg.items()))
        self.cfg = mail.load_config(self.config)

    def enqueue(self, **changes):
        return mail.enqueue(self.cfg, self.config, self.home, 100, self.event | changes)

    def job(self, key):
        with mail.database(self.cfg) as conn:
            payload, state, attempts, error = conn.execute('SELECT payload,state,attempts,error FROM jobs WHERE key=?', (key,)).fetchone()
            return json.loads(payload), state, attempts, error

    def test_goal_marked_once_and_payload_contains_no_chat(self):
        with sqlite3.connect(self.home / 'goals_1.sqlite') as conn:
            conn.execute('INSERT INTO thread_goals VALUES (?,?,?,?)', ('session-1','goal-1','complete',150))
        key = self.enqueue()
        self.assertEqual(key, self.enqueue())
        payload = self.job(key)[0]
        self.assertEqual(payload['name'], '中文项目会话')
        self.assertTrue(payload['goal_completed'])
        second = self.enqueue(**{'turn-id':'turn-2'})
        self.assertFalse(self.job(second)[0]['goal_completed'])
        message = mail.mail_message(self.cfg, key, payload)
        self.assertIn('Goal 已完成', str(message['Subject']))
        self.assertIn('中文项目会话', str(message['Subject']))
        self.assertNotIn('PRIVATE', message.as_string())
        self.assertIn('codex24h resume session-1', message.get_content())

    def test_capacity_mail_is_not_labelled_completed_or_goal(self):
        payload = {'session': 'session-1', 'turn': 'capacity', 'name': 'test session',
                   'completed_at': 1000, 'goal_completed': False, 'type': 'model_capacity'}
        message = mail.mail_message(self.cfg, 'capacity-test', payload)
        self.assertIn('[模型容量不足]', message['Subject'])
        self.assertIn('发生时间', message.get_content())
        self.assertIn('codex24h resume session-1', message.get_content())
        self.assertNotIn('本轮已完成', message.get_content())
        self.assertNotIn('原生完成事件', message.get_content())

    def test_old_goal_on_resume_is_not_a_new_completion(self):
        with sqlite3.connect(self.home / 'goals_1.sqlite') as conn:
            conn.execute('INSERT INTO thread_goals VALUES (?,?,?,?)', ('session-1','old','complete',99))
        self.assertFalse(self.job(self.enqueue())[0]['goal_completed'])

    def test_shared_watch_scopes_completion_to_tui_message_and_deduplicates(self):
        history = self.home/'thread_history_1.sqlite'
        with sqlite3.connect(history) as conn:
            conn.execute('CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,first_user_item_id TEXT,status TEXT,completed_at INTEGER)')
            conn.execute('CREATE TABLE thread_items(thread_id TEXT,item_type TEXT,created_at_ms INTEGER,item_json TEXT)')
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                         ('other-session','other-turn','other-message','completed',int(time.time())))
        observer = mail.CompletionWatch(self.home)
        observer.record({'dir':'from_tui','kind':'op','payload':{'UserTurn':{
            'client_user_message_id':'my-message','items':[{'text':'PRIVATE PROMPT'}]}}})
        self.assertEqual(observer.completed(), [])
        with sqlite3.connect(history) as conn:
            conn.execute('INSERT INTO thread_items VALUES (?,?,?,?)',
                         ('session-1','userMessage',int(time.time()*1000),json.dumps({'clientId':'my-message','content':'PRIVATE PROMPT'})))
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                         ('session-1','turn-1','my-message','completed',int(time.time())))
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                         ('session-1','aborted','my-aborted','interrupted',int(time.time())))
        self.assertEqual(observer.completed(), [
            {'type':'agent-turn-complete','thread-id':'session-1','turn-id':'turn-1'}])
        self.assertEqual(observer.completed(), [])

    def test_shared_resume_ignores_completed_history_but_watches_active_turn(self):
        session = '11111111-1111-4111-8111-111111111111'
        history = self.home/'thread_history_1.sqlite'
        with sqlite3.connect(history) as conn:
            conn.execute('CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,first_user_item_id TEXT,status TEXT,completed_at INTEGER)')
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                         (session,'old','old-message','completed',int(time.time())))
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                         (session,'active','new-message','inProgress',None))
        observer = mail.CompletionWatch(self.home, session)
        self.assertEqual(observer.completed(), [])
        with sqlite3.connect(history) as conn:
            conn.execute("UPDATE thread_turns SET status='completed',completed_at=? WHERE turn_id='active'", (int(time.time()),))
        self.assertEqual(observer.completed(), [
            {'type':'agent-turn-complete','thread-id':session,'turn-id':'active'}])

    def test_no_email_for_interrupt_or_subagent(self):
        self.assertIsNone(self.enqueue(type='turn-aborted'))
        with sqlite3.connect(self.home / 'state_5.sqlite') as conn:
            conn.execute('UPDATE threads SET source=?', ('{"subagent":{}}',))
        self.assertIsNone(self.enqueue())

    def test_unknown_or_unreadable_source_never_enqueues(self):
        for source in (None, '', 'unknown', '{"futureSource":{}}', '{"subAgent":{"thread_spawn":{}}}'):
            with self.subTest(source=source), sqlite3.connect(self.home/'state_5.sqlite') as conn:
                conn.execute('UPDATE threads SET source=?', (source,))
            self.assertIsNone(self.enqueue())
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute("UPDATE threads SET source='cli'")
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('BEGIN EXCLUSIVE')
            self.assertIsNone(self.enqueue())
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('DELETE FROM threads')
        self.assertIsNone(self.enqueue())

    def test_newest_metadata_does_not_fall_back_to_stale_main_source(self):
        (self.home/'state_6.sqlite').write_bytes(b'not a SQLite database')
        self.assertIsNone(self.enqueue())

    def test_spawn_edge_blocks_child_even_with_main_source(self):
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT,status TEXT)')
            conn.execute("INSERT INTO thread_spawn_edges VALUES ('parent','session-1','closed')")
        self.assertIsNone(self.enqueue())

    def test_unreadable_spawn_relationship_does_not_allow_main_source(self):
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('CREATE TABLE thread_spawn_edges(future_column TEXT)')
        self.assertIsNone(self.enqueue())

    def test_queued_delivery_rechecks_child_and_unknown_metadata(self):
        for source in ('{"subagent":{}}', None):
            with self.subTest(source=source):
                with sqlite3.connect(self.home/'state_5.sqlite') as conn:
                    conn.execute("UPDATE threads SET source='cli'")
                key = self.enqueue(**{'turn-id':str(source)})
                with sqlite3.connect(self.home/'state_5.sqlite') as conn:
                    conn.execute('UPDATE threads SET source=?', (source,))
                with patch.object(mail, 'deliver') as deliver:
                    mail.worker(self.config, key)
                    mail.worker(self.config, key)
                    deliver.assert_not_called()
                self.assertEqual(self.job(key)[1:3], ('suppressed', 0))

    def test_shared_child_completion_is_filtered_before_queueing(self):
        history = self.home/'thread_history_1.sqlite'
        with sqlite3.connect(history) as conn:
            conn.execute('CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,completed_at INTEGER)')
            conn.execute('CREATE TABLE thread_items(thread_id TEXT,item_type TEXT,created_at_ms INTEGER,item_json TEXT)')
        observer = mail.CompletionWatch(self.home)
        observer.record({'dir':'from_tui','kind':'op','payload':{'UserTurn':{'client_user_message_id':'child-message'}}})
        with sqlite3.connect(history) as conn:
            conn.execute('INSERT INTO thread_items VALUES (?,?,?,?)',
                         ('session-1','userMessage',observer.since,json.dumps({'clientId':'child-message'})))
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?)',
                         ('session-1','child-turn','completed',int(time.time())))
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('UPDATE threads SET source=?', ('{"subagent":{"thread_spawn":{}}}',))
        events = observer.completed()
        self.assertEqual(len(events), 1)
        self.assertIsNone(mail.enqueue(self.cfg,self.config,self.home,observer.since,events[0]))

    def test_capacity_callback_blocks_child_and_missing_metadata(self):
        rollout = self.home/'rollout-fixture-session-1.jsonl'
        rollout.write_text(json.dumps({'type':'session_meta','payload':{'id':'session-1'}})+'\n')
        for source in ('{"subagent":{"thread_spawn":{}}}', None):
            with sqlite3.connect(self.home/'state_5.sqlite') as conn:
                conn.execute('UPDATE threads SET source=?', (source,))
            with rollout.open():
                subprocess.run([sys.executable,str(SCRIPT),'capacity','--config',str(self.config),'--pid',str(os.getpid())],
                               env=os.environ|{'CODEX_HOME':str(self.home)},check=True,timeout=5,
                               stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
        with mail.database(self.cfg) as conn:
            self.assertEqual(conn.execute('SELECT count(*) FROM jobs').fetchone()[0],0)

    def test_legacy_queued_child_is_blocked_with_inherited_codex_home(self):
        key = self.enqueue()
        payload = self.job(key)[0]
        payload.pop('codex_home')
        with mail.database(self.cfg) as conn:
            conn.execute('UPDATE jobs SET payload=? WHERE key=?',(json.dumps(payload),key))
            conn.commit()
        with sqlite3.connect(self.home/'state_5.sqlite') as conn:
            conn.execute('UPDATE threads SET source=?', ('{"subagent":{}}',))
        with patch.dict(os.environ,{'CODEX_HOME':str(self.home)}), patch.object(mail,'deliver') as deliver:
            mail.worker(self.config,key)
            deliver.assert_not_called()
        self.assertEqual(self.job(key)[1:3], ('suppressed',0))

    def test_failure_is_visible_and_retry_deduplicates(self):
        key = self.enqueue()
        with patch.object(mail, 'deliver', side_effect=smtplib.SMTPAuthenticationError(535,b'PRIVATE SECRET')):
            mail.worker(self.config, key)
        _, state, attempts, error = self.job(key)
        self.assertEqual((state,attempts,error), ('failed',1,'SMTPAuthenticationError (535)'))
        with patch.object(mail, 'deliver') as deliver:
            mail.worker(self.config, key)
            mail.worker(self.config, key)
            self.assertEqual(deliver.call_count,1)
        self.assertEqual(self.job(key)[1], 'sent')

    def test_other_mail_config_cannot_send_this_queue_entry(self):
        key = self.enqueue()
        other = self.base/'other.toml'
        other.write_text(self.config.read_text())
        with patch.object(mail, 'deliver') as deliver:
            mail.worker(other, key)
            deliver.assert_not_called()
        self.assertEqual(self.job(key)[1], 'pending')

    def test_broken_previous_notifier_does_not_block_email(self):
        args = SimpleNamespace(config=self.config, codex_home=self.home, since=100,
                               previous='["missing-notifier"]', event=json.dumps(self.event))
        errors = io.StringIO()
        with patch.object(mail.subprocess, 'Popen', side_effect=FileNotFoundError('PRIVATE')):
            with patch.object(mail, 'spawn_worker') as spawn, contextlib.redirect_stderr(errors):
                mail.notify(args)
                spawn.assert_called_once()
        self.assertNotIn('PRIVATE', errors.getvalue())
        self.assertEqual(self.job(self.enqueue())[1], 'pending')

    def test_profile_and_cli_notify_are_preserved_without_config_edits(self):
        original = 'notify = ["global"]\nprofile = "work"\n'
        (self.home/'config.toml').write_text(original)
        (self.home/'work.config.toml').write_text('notify = ["profile", "参数"]\n')
        self.assertEqual(mail.previous_notify(self.home, []), ['profile','参数'])
        args = ['resume','--last','-c','notify=["cli", "a b"]']
        output = subprocess.check_output(['python3',str(SCRIPT),'prepare','--config',str(self.config),
                                         '--codex-home',str(self.home),'--',*args],text=True)
        command = tomllib.loads(output)['notify']
        previous = json.loads(command[command.index('--previous')+1])
        self.assertEqual(previous, ['cli','a b'])
        self.assertEqual((self.home/'config.toml').read_text(),original)

    def test_private_password_and_required_tls(self):
        self.write_config(username='sender', password_file='secret')
        secret = self.base/'secret'
        secret.write_text('example-auth-code')
        secret.chmod(0o644)
        with self.assertRaisesRegex(ValueError,'private'):
            mail.password(self.cfg)
        secret.chmod(0o600)
        self.assertEqual(mail.password(self.cfg),'example-auth-code')
        with self.assertRaisesRegex(ValueError,'TLS'):
            self.write_config(security='none')

    def test_starttls_failure_never_sends_credentials(self):
        self.write_config(security='starttls',username='test-sender')
        key = self.enqueue()
        message = mail.mail_message(self.cfg,key,self.job(key)[0])
        with patch.object(mail,'password',return_value='example-secret'):
            with patch.object(mail.smtplib,'SMTP') as smtp:
                client = smtp.return_value
                client.starttls.side_effect = smtplib.SMTPNotSupportedError('No TLS')
                with self.assertRaises(smtplib.SMTPNotSupportedError):
                    mail.deliver(self.cfg,message)
                client.login.assert_not_called()
                client.send_message.assert_not_called()
                client.close.assert_called_once()

    def test_wrapper_injects_callback_for_sessions_and_preserves_literal_prompt(self):
        binary = Path(os.environ.get('CODEX24H_TEST_BIN',ROOT/'target/debug/codex24h'))
        fake = self.base/'fake-codex'
        fake.write_text('#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n')
        fake.chmod(0o755)
        env = os.environ | {'CODEX24H_CODEX':str(fake),'CODEX24H_MAIL_CONFIG':str(self.config),'CODEX_HOME':str(self.home)}
        for args in (['resume','--last'], ['exec','--','literal prompt'], ['--yolo','exec','hello']):
            result = json.loads(subprocess.check_output([str(binary),*args],env=env,text=True))
            index = next(i for i,arg in enumerate(result) if arg.startswith('notify='))
            self.assertEqual(result[index-1],'-c')
            original = result[:index-1]+result[index+1:]
            self.assertEqual(original,args)
            if '--' in result:
                self.assertLess(index,result.index('--'))
        env['CODEX24H_MAIL_CONFIG'] = str(self.base/'missing-config')
        result = json.loads(subprocess.check_output([str(binary),'resume','--help'],env=env,text=True))
        self.assertEqual(result,['resume','--help'])

    def test_background_callback_sends_real_tls_mail_once(self):
        cert,key = self.base/'cert.pem',self.base/'key.pem'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
                        '-keyout',str(key),'-out',str(cert),'-subj','/CN=localhost',
                        '-addext','subjectAltName=DNS:localhost'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        with TLSService(cert,key) as server:
            thread = threading.Thread(target=server.serve_forever,daemon=True)
            thread.start()
            self.addCleanup(server.shutdown)
            self.write_config(port=server.server_address[1])
            env = os.environ | {'SSL_CERT_FILE':str(cert)}
            args = ['python3',str(SCRIPT),'notify','--config',str(self.config),
                    '--codex-home',str(self.home),'--since','100',json.dumps(self.event)]
            for source in ('{"subagent":{"thread_spawn":{}}}', None):
                with sqlite3.connect(self.home/'state_5.sqlite') as conn:
                    conn.execute('UPDATE threads SET source=?', (source,))
                subprocess.run(args,env=env,check=True,timeout=5)
            time.sleep(.2)
            self.assertTrue(server.messages.empty(), 'child or unknown-source callback sent email')
            self.assertFalse((self.state/'deliveries.sqlite').exists(), 'blocked callbacks created a queue')
            with sqlite3.connect(self.home/'state_5.sqlite') as conn:
                conn.execute("UPDATE threads SET source='cli'")
            started = time.monotonic()
            subprocess.run(args,env=env,check=True,timeout=5)
            self.assertLess(time.monotonic()-started,.65,'callback waited for slow SMTP')
            raw = server.messages.get(timeout=8)
            message = BytesParser(policy=policy.default).parsebytes(raw)
            self.assertIn('中文项目会话',str(message['Subject']))
            self.assertIn('本轮已完成',str(message['Subject']))
            self.assertNotIn('PRIVATE',message.get_content())
            key = self.enqueue()
            end = time.monotonic()+3
            while self.job(key)[1] != 'sent' and time.monotonic()<end:
                time.sleep(.05)
            self.assertEqual(self.job(key)[1],'sent')
            subprocess.run(args,env=env,check=True,timeout=5)
            time.sleep(.2)
            self.assertTrue(server.messages.empty())
            self.assertEqual((self.state/'deliveries.sqlite').stat().st_mode & 0o077,0)


if __name__ == '__main__':
    unittest.main(verbosity=2)
