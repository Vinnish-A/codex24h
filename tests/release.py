#!/usr/bin/env python3
"""Standalone bundle smoke and real local TLS SMTP regression; no external email."""
import json
import os
from pathlib import Path
import queue
import subprocess
import sqlite3
import stat
import sys
import threading
import time
import e2e
from email import policy
from email.parser import BytesParser
import mail as regression

bundle = Path(sys.argv[1]).resolve()
case = regression.MailTests('test_background_callback_sends_real_tls_mail_once')
case.setUp()
try:
    env = os.environ | {'PATH': '/nonexistent', 'CODEX_HOME': str(case.home)}
    helper = str(bundle / 'codex24h-mail')
    for name in ('mail', 'attach', 'session'):
        subprocess.run([str(bundle / ('codex24h-' + name)), '--help'], env=env,
                       check=True, stdout=subprocess.DEVNULL)
    cert, key = case.base / 'cert.pem', case.base / 'key.pem'
    subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                    '-keyout', str(key), '-out', str(cert), '-subj', '/CN=localhost',
                    '-addext', 'subjectAltName=DNS:localhost'], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    with regression.TLSService(cert, key) as server:
        threading.Thread(target=server.serve_forever, daemon=True).start()
        def message_bytes():
            try:
                return server.messages.get(timeout=20)
            except queue.Empty as error:
                with sqlite3.connect(case.state/'deliveries.sqlite') as conn:
                    outcomes = conn.execute('SELECT state,error,count(*) FROM jobs GROUP BY state,error').fetchall()
                raise AssertionError('SMTP delivery timed out; queue outcomes: '+repr(outcomes)) from error
        try:
            case.write_config(port=server.server_address[1], ca_file=str(cert))
            args = [helper, 'notify', '--config', str(case.config), '--codex-home', str(case.home),
                    '--since', '100', '--previous', '[]', json.dumps(case.event)]
            subprocess.run(args, env=env, check=True, timeout=5)
            msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
            assert '本轮已完成' in str(msg['Subject'])
            assert '中文项目会话' in str(msg['Subject'])
            assert 'PRIVATE' not in msg.get_content()
            subprocess.run(args, env=env, check=True, timeout=5)
            assert server.messages.empty(), 'duplicate completion delivery'
            # The new shared-mode adapter must also work inside the bundled runtime.
            session = '11111111-1111-4111-8111-111111111111'
            with sqlite3.connect(case.home/'state_5.sqlite') as conn:
                conn.execute('INSERT INTO threads VALUES (?,?,?,?)', (session, '共享模式测试', 'test', 'cli'))
            with sqlite3.connect(case.home/'thread_history_1.sqlite') as conn:
                conn.execute('CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,completed_at INTEGER,error_json TEXT)')
                conn.execute('CREATE TABLE thread_items(thread_id TEXT,item_type TEXT,created_at_ms INTEGER,item_json TEXT)')
            watcher = subprocess.Popen([helper, 'watch', '--config', str(case.config), '--codex-home', str(case.home)],
                                       env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            pipe = None
            try:
                pipe = Path(watcher.stdout.readline().strip())
                assert stat.S_ISFIFO(pipe.stat().st_mode), 'event recording must be a FIFO'
                with sqlite3.connect(case.home/'thread_history_1.sqlite') as conn:
                    conn.execute('INSERT INTO thread_items VALUES (?,?,?,?)',
                                 (session, 'userMessage', int(time.time()*1000), json.dumps({'clientId':'bundle-message'})))
                    conn.execute('INSERT INTO thread_turns(thread_id,turn_id,status,completed_at) VALUES (?,?,?,?)',
                                 (session, 'bundle-turn', 'completed', int(time.time())))
                event = {'dir':'from_tui', 'kind':'op', 'payload':{'UserTurn':{
                    'client_user_message_id':'bundle-message', 'items':[{'text':'PRIVATE PROMPT'}]}}}
                with pipe.open('w') as stream:
                    stream.write(json.dumps(event)+'\n')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert session in msg.get_content()
                assert '共享模式测试' in str(msg['Subject'])
                assert 'PRIVATE' not in msg.get_content()
                with pipe.open('w') as stream:
                    stream.write(json.dumps(event)+'\n')
                time.sleep(.5)
                assert server.messages.empty(), 'duplicate shared completion delivery'
                with sqlite3.connect(case.home/'thread_history_1.sqlite') as conn:
                    conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)',
                                 (session, 'shared-capacity', 'failed', int(time.time()),
                                  json.dumps({'codexErrorInfo':'serverOverloaded','message':'PRIVATE CAPACITY DETAILS'})))
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject'])
                assert session in msg.get_content()
                assert 'PRIVATE' not in msg.get_content()
                time.sleep(.5)
                assert server.messages.empty(), 'duplicate shared capacity delivery'
            finally:
                watcher.terminate()
                watcher.wait(timeout=5)
                watcher.stdout.close()
                watcher_error = watcher.stderr.read()
                watcher.stderr.close()
                assert watcher.returncode == 0, 'shared watcher exited: '+watcher_error
                if pipe is not None:
                    assert not pipe.exists(), 'watcher event pipe was not cleaned up'
            rollout = case.home / 'rollout-fixture-session-1.jsonl'
            rollout.write_text(json.dumps({'type': 'session_meta', 'payload': {'id': 'session-1'}}) + '\n' +
                               json.dumps({'type': 'event_msg', 'payload': {'type': 'turn_started', 'turn_id': 'turn-capacity'}}) + '\n')
            with rollout.open():
                subprocess.run([helper, 'capacity', '--pid', str(os.getpid()), '--config', str(case.config)],
                               env=env, check=True, timeout=8)
            msg = BytesParser(policy=policy.default).parsebytes(server.messages.get(timeout=3))
            assert '模型容量不足' in str(msg['Subject'])
            assert '中文项目会话' in str(msg['Subject'])
            assert '原生完成事件' not in msg.get_content()
            with rollout.open():
                subprocess.run([helper, 'capacity', '--pid', str(os.getpid()), '--config', str(case.config)],
                               env=env, check=True, timeout=8)
            assert server.messages.empty(), 'duplicate capacity delivery in the same turn'
            fake = case.base / 'fake-capacity'
            fake.write_text('#!' + sys.executable + '\n' + """
import json, os, sys
from pathlib import Path
p = Path(os.environ['CODEX_HOME']) / 'rollout-native-session-1.jsonl'
f = p.open('w')
f.write(json.dumps({'type':'session_meta','payload':{'id':'session-1'}})+'\\n'); f.flush()
turn = 0
print('CAPACITY-READY', flush=True)
for line in sys.stdin:
    if line.strip() == 'next':
        turn += 1
        f.write(json.dumps({'type':'event_msg','payload':{'type':'turn_started','turn_id':'native-'+str(turn)}})+'\\n'); f.flush()
        print('\\x1b[31m■ Selected model is at capacity. Please try a different model.\\x1b[0m', flush=True)
    elif line.strip() == 'repaint':
        print('\\x1b[31m■ Selected model is at capacity. Please try a different model.\\x1b[0m', flush=True)
    elif line.strip() == 'quote':
        print('› Selected model is at capacity', flush=True)
    elif line.strip() == 'clear':
        print('\\x1b[2J\\x1b[Hcleared', flush=True)
    elif line.strip() == 'exit':
        break
""")
            fake.chmod(0o755)
            e2e.BIN = bundle / 'codex24h'
            native_env = env | {'CODEX24H_CODEX': str(fake), 'CODEX24H_MAIL_CONFIG': str(case.config),
                                'CODEX24H_MAIL': '1', 'TERM': 'xterm-256color'}
            terminal = e2e.Terminal(native_env)
            try:
                terminal.until('CAPACITY-READY')
                terminal.send(b'quote\r'); terminal.drain(.3)
                assert server.messages.empty(), 'quoted capacity text sent a notification'
                terminal.send(b'next\r')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject'])
                terminal.send(b'repaint\r'); terminal.drain(.5)
                assert server.messages.empty(), 'native redraw sent a duplicate notification'
                terminal.send(b'clear\r'); terminal.drain(.2)
                terminal.send(b'next\r')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject'])
                terminal.send(b'exit\r'); terminal.process.wait(timeout=5)
            finally:
                terminal.close()
            # Shared frontends have no rollout file descriptor. Exercise the real
            # wrapper, FIFO, SQLite errors and TLS sender together, including /resume.
            resumed = '22222222-2222-4222-8222-222222222222'
            with sqlite3.connect(case.home/'state_5.sqlite') as conn:
                conn.execute('INSERT INTO threads VALUES (?,?,?,?)', (resumed, '恢复容量测试', 'test', 'cli'))
            fake_shared = case.base/'fake-shared-capacity'
            fake_shared.write_text('#!'+sys.executable+'\n'+"""
import json, os, sys, sqlite3, time, tty
from pathlib import Path
if '--help' in sys.argv:
    print('--no-daemon'); raise SystemExit(0)
tty.setraw(0)
home=Path(os.environ['CODEX_HOME'])
session='11111111-1111-4111-8111-111111111111'
resumed='22222222-2222-4222-8222-222222222222'
pipe=Path(os.environ['CODEX_TUI_SESSION_LOG_PATH'])
def fail(target, turn, client=None):
    with sqlite3.connect(home/'thread_history_1.sqlite') as conn:
        if client:
            conn.execute('INSERT INTO thread_items VALUES (?,?,?,?)', (target,'userMessage',int(time.time()*1000),json.dumps({'clientId':client})))
        conn.execute('INSERT INTO thread_turns VALUES (?,?,?,?,?)', (target,turn,'failed',int(time.time()),json.dumps({'message':'Selected model is at capacity','codexErrorInfo':'serverOverloaded'})))
    if client:
        with pipe.open('w') as stream:
            stream.write(json.dumps({'dir':'from_tui','kind':'op','payload':{'UserTurn':{'client_user_message_id':client}}})+'\\n')
    # Deliberately uncolored: shared detection must use structured errors.
    print('\\x1b[2J\\x1b[H■ Selected model is at capacity.', flush=True)
print('SHARED-CAPACITY-READY',flush=True)
pending=b''
picker=False
turn=0
while data:=os.read(0,4096):
    pending+=data
    while b'\\r' in pending:
        line,pending=pending.split(b'\\r',1)
        if picker:
            fail(resumed,'resumed-capacity'); picker=False
        elif line==b'next':
            turn+=1; fail(session,'native-shared-'+str(turn),'native-capacity-'+str(turn))
        elif line==b'/resume':
            print('\\x1b[2J\\x1b[HResume a previous session\\r\\n│ Session: test ('+resumed+')\\r\\n│ Created: today\\r\\n enter resume',flush=True)
            picker=True
        elif line==b'repaint':
            print('\\x1b[2J\\x1b[H\\x1b[31m■ Selected model is at capacity.\\x1b[0m',flush=True)
        elif line==b'exit':
            raise SystemExit(0)
""")
            fake_shared.chmod(0o755)
            shared_env = native_env | {'CODEX24H_CODEX':str(fake_shared)}
            terminal = e2e.Terminal(shared_env)
            try:
                terminal.until('SHARED-CAPACITY-READY')
                terminal.send(b'next\r')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject']) and session in msg.get_content()
                terminal.send(b'repaint\r'); terminal.drain(.5)
                assert server.messages.empty(), 'shared screen repaint sent duplicate capacity mail'
                terminal.send(b'next\r')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject'])
                terminal.send(b'/resume\r')
                deadline = time.monotonic()+5
                while time.monotonic()<deadline:
                    terminal.drain(.05)
                    if any('Resume a previous session' in line for line in e2e.screen_text(terminal.output)):
                        break
                else:
                    raise AssertionError('shared native resume picker did not appear')
                terminal.send(b'\r')
                msg = BytesParser(policy=policy.default).parsebytes(message_bytes())
                assert '模型容量不足' in str(msg['Subject']) and resumed in msg.get_content()
                terminal.send(b'exit\r'); terminal.wait_draining()
            finally:
                terminal.close()
            print('PASS bundled runtime without Python on PATH: helpers, TLS SMTP, shared completion/capacity, runtime resume, FIFO cleanup, deduplication and standalone capacity')
        finally:
            server.shutdown()
finally:
    case.doCleanups()
