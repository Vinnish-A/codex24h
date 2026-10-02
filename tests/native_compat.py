#!/usr/bin/env python3
"""Native multi-version resume/agents checks. --mail sends one test prompt per CLI."""
import argparse
from email import policy
from email.parser import BytesParser
import json
import os
from pathlib import Path
import queue
import shlex
import subprocess
import tempfile
import threading
import time

import e2e
from mail import TLSService


def run(binary, codex, session, smtp, config, socket, standalone=False):
    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], text=True)

    cmd = ['env', 'CODEX24H_MAIL_CONFIG='+str(config), 'CODEX24H_CODEX='+codex,
           str(binary), *(['--no-daemon'] if standalone else []), *(['resume', session] if session else [])]
    pane = tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'probe', '-x', '110', '-y', '34',
                '-P', '-F', '#{pane_id}', '-c', str(e2e.ROOT), shlex.join(cmd)).strip()
    tmux('set-option', '-w', '-t', pane, 'remain-on-exit', 'on')
    def screen():
        return tmux('capture-pane', '-p', '-t', pane)

    def wait(predicate, label, timeout=90):
        deadline = time.monotonic()+timeout
        while time.monotonic() < deadline:
            text = screen()
            if predicate(text):
                return text
            if 'Update available' in text and 'esc skip' in text.lower():
                tmux('send-keys', '-t', pane, 'Escape')
            elif 'Working directory' in text and 'esc use session' in text.lower():
                tmux('send-keys', '-t', pane, 'Enter')
            time.sleep(.2)
        raise AssertionError(label+' timed out:\n'+screen())

    version = subprocess.check_output([codex, '--version'], text=True).strip()
    try:
        if standalone:
            wait(lambda text: 'Ask Codex' in text and 'gpt-' in text.lower(), 'standalone native composer')
        else:
            wait(lambda text: '← for agents' in text, 'shared native composer')
            tmux('send-keys', '-t', pane, 'Left')
            wait(lambda text: 'Ask Codex' not in text and 'agent' in text.lower(), 'agents overview')
            tmux('send-keys', '-t', pane, 'Escape')
            wait(lambda text: '← for agents' in text, 'return from agents')
        tmux('send-keys', '-t', pane, '-l', '/mo')
        wait(lambda text: '/model' in text, 'native completion')
        tmux('send-keys', '-t', pane, 'Tab')
        wait(lambda text: '› /model' in text, 'native Tab')
        tmux('send-keys', '-t', pane, 'C-u')
        if smtp:
            tmux('send-keys', '-t', pane, '-l', '只回复 COMPAT_MAIL_OK，不要调用工具。')
            time.sleep(.3)
            tmux('send-keys', '-t', pane, 'Enter')
            raw = smtp.messages.get(timeout=90)
            message = BytesParser(policy=policy.default).parsebytes(raw)
            assert '本轮已完成' in message['Subject']
            assert 'Session ID：' in message.get_content()
            if session:
                assert session in message.get_content()
            body = '\n'.join(line for line in message.get_content().splitlines() if not line.startswith('会话：'))
            assert 'COMPAT_MAIL_OK' not in body
        print(json.dumps({'version': version, 'resume': bool(session), 'agents': not standalone,
                          'completion': True, 'mail': bool(smtp)}, ensure_ascii=False), flush=True)
    finally:
        # A terminated client disconnects; never interrupt an existing user's turn.
        pid = int(tmux('display-message', '-p', '-t', pane, '#{pane_pid}'))
        import signal
        if Path(f'/proc/{pid}').exists():
            os.kill(pid, signal.SIGTERM)
        time.sleep(.3)
        subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=e2e.BIN)
    parser.add_argument('--codex', action='append', required=True)
    parser.add_argument('--session', help='existing idle session; omitted creates a test session')
    parser.add_argument('--mail', action='store_true')
    parser.add_argument('--no-daemon', action='store_true')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='codex24h-compat-') as directory:
        base = Path(directory)
        cert, key = base/'cert.pem', base/'key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                        '-keyout', str(key), '-out', str(cert), '-subj', '/CN=localhost',
                        '-addext', 'subjectAltName=DNS:localhost'], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with TLSService(cert, key) as smtp:
            threading.Thread(target=smtp.serve_forever, daemon=True).start()
            try:
                config = base/'mail.toml'
                cfg = {'enabled': True, 'host': 'localhost', 'port': smtp.server_address[1],
                       'security': 'ssl', 'from': 'test@example.com', 'to': ['test@example.com'],
                       'state_dir': str(base/'state'), 'ca_file': str(cert)}
                config.write_text('\n'.join(f'{k} = {json.dumps(v)}' for k,v in cfg.items()))
                for codex in args.codex:
                    run(args.binary, codex, args.session, smtp if args.mail else None,
                        config, str(base/'tmux.sock'), args.no_daemon)
            finally:
                smtp.shutdown()


if __name__ == '__main__':
    main()
