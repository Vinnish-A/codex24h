#!/usr/bin/env python3
"""Opt-in real Codex capacity recovery against local SSE + TLS SMTP only.

Run CODEX24H_CODEX=/path/to/codex python3 tests/native_capacity.py [--standalone].
Uses an isolated CODEX_HOME and daemon; never contacts a real model or SMTP host.
"""
from email import policy
from email.parser import BytesParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import e2e
from mail import TLSService, ROOT


class CapacityHandler(BaseHTTPRequestHandler):
    def log_message(self,*args):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers['Content-Length']))
        self.server.calls.append(time.monotonic())
        event = {'type':'response.failed','response':{'id':'local_capacity',
                 'error':{'code':'server_is_overloaded','message':'Selected model is at capacity.'}}}
        data = ('event: response.failed\ndata: '+json.dumps(event)+'\n\n').encode()
        self.send_response(200)
        self.send_header('Content-Type','text/event-stream')
        self.send_header('Content-Length',str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def main():
    codex = os.environ['CODEX24H_CODEX']
    standalone = '--standalone' in sys.argv
    e2e.BIN = Path(os.environ.get('CODEX24H_TEST_BIN',ROOT/'target/debug/codex24h'))
    with tempfile.TemporaryDirectory(prefix='codex24h-native-capacity-') as directory:
        base=Path(directory);home=base/'codex';home.mkdir()
        cert,key=base/'cert.pem',base/'key.pem'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
                        '-keyout',str(key),'-out',str(cert),'-subj','/CN=localhost',
                        '-addext','subjectAltName=DNS:localhost'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        with TLSService(cert,key) as smtp, ThreadingHTTPServer(('127.0.0.1',0),CapacityHandler) as model:
            model.calls=[]
            for server in (smtp,model):
                threading.Thread(target=server.serve_forever,daemon=True).start()
            config=base/'mail.toml'
            cfg=dict(enabled=True,host='localhost',port=smtp.server_address[1],security='ssl',
                     to=['test@example.com'],state_dir=str(base/'state'),ca_file=str(cert))
            cfg['from']='test@example.com'
            config.write_text('\n'.join(f'{k} = {json.dumps(v)}' for k,v in cfg.items()))
            (home/'config.toml').write_text(f'''
model = "local-capacity-model"
model_provider = "mock"
[features]
goals = true
api_key_model_discovery = false
[projects.{json.dumps(str(ROOT))}]
trust_level = "trusted"
[model_providers.mock]
name = "Local capacity test"
base_url = "http://127.0.0.1:{model.server_port}/v1"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
''')
            env=os.environ|{'CODEX_HOME':str(home),'CODEX24H_CODEX':codex,'CODEX24H_MAIL_CONFIG':str(config)}
            daemon=None;terminal=None
            try:
                if not standalone:
                    sock=home/'app-server-control/app-server-control.sock';sock.parent.mkdir()
                    daemon=subprocess.Popen([codex,'app-server','--managed-daemon','--listen','unix://'+str(sock)],
                        env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    deadline=time.monotonic()+10
                    while not sock.exists():
                        assert daemon.poll() is None and time.monotonic()<deadline,'isolated daemon did not start'
                        time.sleep(.05)
                args=['--dangerously-bypass-approvals-and-sandbox']
                if standalone:args.append('--no-daemon')
                terminal=e2e.Terminal(env,args)
                terminal.until('Ask Codex',timeout=20)
                terminal.drain(1)

                def submit(text):
                    terminal.send(b'\x1b[200~'+text.encode()+b'\x1b[201~')
                    terminal.drain(.3)
                    terminal.send(b'\r')

                def wait_for(predicate,timeout=65):
                    deadline=time.monotonic()+timeout
                    while time.monotonic()<deadline:
                        terminal.drain(.1)
                        if predicate():return
                    raise AssertionError('native capacity check timed out; '+e2e.visible(terminal.output)[-1400:])

                def mails(number):
                    found=[]
                    wait_for(lambda:collect(found,number))
                    return found

                def collect(found,number):
                    while True:
                        try:found.append(BytesParser(policy=policy.default).parsebytes(smtp.messages.get_nowait()))
                        except queue.Empty:break
                    return len(found)>=number

                submit('Local capacity recovery test.')
                received=mails(4)
                assert all('自动恢复请求已提交' in str(m['Subject']) for m in received[:3])
                assert '容量不足，请检查' in str(received[3]['Subject'])
                with sqlite3.connect(home/'thread_history_1.sqlite') as conn:
                    turns=conn.execute('SELECT turn_id,status FROM thread_turns ORDER BY rowid').fetchall()
                    inputs=[json.loads(row[0]) for row in conn.execute("SELECT item_json FROM thread_items WHERE item_type='userMessage' ORDER BY rowid")]
                assert len(turns)==4 and all(status=='failed' for _,status in turns),repr(turns)
                replies=[''.join(part.get('text','') for part in item['content']) for item in inputs]
                assert replies.count('go on')==3,repr(replies)
                terminal.drain(.5)
                assert smtp.messages.empty()
                print('PASS real Codex: independent jitter, three go on submissions, fourth check mail',flush=True)

                # A manual Goal command has no clientId. Identity must come from
                # this TUI's /status, and native blocked must not be treated paused.
                submit('/goal Local Goal capacity recovery test.')
                notice=mails(1)[0]
                assert '自动恢复受阻' in str(notice['Subject'])
                assert '不是 paused' in notice.get_content()
                with sqlite3.connect(home/'goals_1.sqlite') as conn:
                    session,status=conn.execute('SELECT thread_id,status FROM thread_goals').fetchone()
                assert status=='blocked'
                submit('/goal pause')

                def paused():
                    with sqlite3.connect(home/'goals_1.sqlite') as conn:
                        row=conn.execute('SELECT status FROM thread_goals WHERE thread_id=?',(session,)).fetchone()
                    return row and row[0]=='paused'
                wait_for(paused,10)
                terminal.drain(2)
                # Persist a fresh, isolated capacity failure while the actual TUI
                # holds a paused Goal. The next turn is driven by native /goal resume.
                with sqlite3.connect(home/'thread_history_1.sqlite') as conn:
                    ordinal=conn.execute('SELECT max(rollout_ordinal) FROM thread_turns WHERE thread_id=?',(session,)).fetchone()[0]+10
                    conn.execute('''INSERT INTO thread_turns(thread_id,turn_id,rollout_ordinal,status,error_json,started_at,completed_at)
                                    VALUES (?,?,?,'failed',?,?,?)''',
                        (session,str(uuid.uuid4()),ordinal,json.dumps({'codexErrorInfo':'serverOverloaded'}),int(time.time()),int(time.time())))
                found=[]
                wait_for(lambda:collect(found,1),30)
                assert '自动恢复请求已提交' in str(found[0]['Subject']),found[0].get_content()
                assert '/goal resume' in found[0].get_content()
                assert '第 1/3 次' in found[0].get_content(),'manual new Goal must reset the budget'
                with sqlite3.connect(home/'thread_history_1.sqlite') as conn:
                    count=conn.execute('SELECT count(*) FROM thread_turns').fetchone()[0]
                assert count==7,'four ordinary turns, Goal creation, synthetic failure and one native resume: '+str(count)
                print('PASS real Codex: fresh Goal identity, blocked guard, native paused Goal resume and confirmed mail',flush=True)
            finally:
                if terminal:terminal.close()
                if daemon:
                    daemon.kill();daemon.wait(timeout=5)
                smtp.shutdown();model.shutdown()


if __name__=='__main__':
    main()
