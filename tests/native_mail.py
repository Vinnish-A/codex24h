#!/usr/bin/env python3
"""Opt-in native completion -> local TLS SMTP check. Sends two real model prompts."""
from email import policy
from email.parser import BytesParser
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
import time
import queue

import e2e

from mail import ROOT, TLSService


def main():
    with tempfile.TemporaryDirectory(prefix='codex24h-native-mail-') as directory:
        base = Path(directory)
        cert, key = base/'cert.pem', base/'key.pem'
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1',
                        '-keyout',str(key),'-out',str(cert),'-subj','/CN=localhost',
                        '-addext','subjectAltName=DNS:localhost'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        config = base/'mail.toml'
        with TLSService(cert,key) as server:
            thread = threading.Thread(target=server.serve_forever,daemon=True)
            thread.start()
            try:
                cfg = {'enabled':True,'host':'localhost','port':server.server_address[1], 'security':'ssl',
                       'from':'test@example.com','to':['test@example.com'],'state_dir':str(base/'state'), 'ca_file':str(cert)}
                config.write_text('\n'.join(f'{k} = {json.dumps(v)}' for k,v in cfg.items()))
                binary = os.environ.get('CODEX24H_TEST_BIN',str(ROOT/'target/debug/codex24h'))
                env = os.environ | {'CODEX24H_MAIL_CONFIG':str(config)}
                result = subprocess.run([binary,'exec','--skip-git-repo-check','--json',
                                         '仅回复 MAIL_NOTIFY_OK，不要调用工具，不要读写文件。'],
                                        cwd=ROOT,env=env,capture_output=True,text=True,timeout=120)
                if result.returncode:
                    raise RuntimeError(f'native Codex exited {result.returncode}; stderr retained only in memory')
                session = None
                for line in result.stdout.splitlines():
                    event = json.loads(line)
                    if event.get('type') == 'thread.started':
                        session = event['thread_id']
                assert session, 'Codex did not report its session ID'
                raw = server.messages.get(timeout=20)
                message = BytesParser(policy=policy.default).parsebytes(raw)
                assert session in message.get_content(), 'email did not identify the actual native session'
                assert '本轮已完成' in str(message['Subject']), 'ordinary turn mislabeled'
                without_name = '\n'.join(line for line in message.get_content().splitlines() if not line.startswith('会话：'))
                assert 'MAIL_NOTIFY_OK' not in without_name, 'assistant body leaked into notification'
                print('PASS: native exec completion automatically delivered TLS mail with actual session ID and session name', flush=True)
                print(f'Test session: {session}')
                # Resume that same test-created session in the actual PTY wrapper.
                e2e.BIN = Path(binary)
                terminal = e2e.Terminal(env, ('resume', session))
                try:
                    terminal.until('Ask Codex', timeout=45)
                    terminal.until('MAIL_NOTIFY_OK', timeout=45)
                    terminal.drain(1)
                    terminal.send('仅回复 SECOND_MAIL_OK，不要调用工具，不要读写文件。'.encode())
                    terminal.drain(.4)
                    terminal.send(b'\r')
                    deadline = time.monotonic()+90
                    raw = None
                    while time.monotonic()<deadline:
                        terminal.drain(.1)
                        try:
                            raw = server.messages.get_nowait()
                            break
                        except queue.Empty:
                            pass
                    if not raw:
                        print('Test TUI tail:', e2e.visible(terminal.output)[-1800:])
                        raise AssertionError('resumed native TUI did not produce a completion email')
                    second = BytesParser(policy=policy.default).parsebytes(raw)
                    assert session in second.get_content()
                    assert second['Message-ID'] != message['Message-ID']
                    print('PASS: resumed native TUI turn automatically delivered a second email', flush=True)
                finally:
                    terminal.close()
            finally:
                server.shutdown()


if __name__ == '__main__':
    main()
