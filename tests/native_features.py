#!/usr/bin/env python3
"""Real Codex: pinned editing/completion. Uses existing login."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import sys
import time

import e2e


def main():
    with tempfile.TemporaryDirectory(prefix='codex24h-native-features-') as directory:
        base = Path(directory)
        socket = str(base/'tmux.sock')
        def tmux(*args):
            return subprocess.check_output(['tmux', '-S', socket, *args], text=True).strip()
        attach = '--attach' in sys.argv
        executable = ['codex', '--no-alt-screen', '-c', 'notify=[]'] if attach else [str(e2e.BIN)]
        cmd = ['env', 'CODEX24H_MAIL=0', 'CODEX24H_PIN_ROWS=6',
               *executable]
        pane, pid = tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'probe', '-x', '100', '-y', '32',
                         '-c', str(e2e.ROOT), '-P', '-F', '#{pane_id} #{pane_pid}', shlex.join(cmd)).split()
        def screen():
            return tmux('capture-pane', '-p', '-t', pane)
        def send(data):
            tmux('send-keys', '-t', pane, '-H', *[f'{byte:02x}' for byte in data])
        def wait(predicate, timeout=90):
            end = time.monotonic()+timeout
            while time.monotonic()<end:
                text = screen()
                if predicate(text): return text
                time.sleep(.1)
            raise AssertionError('native feature timeout:\n'+screen())
        def enter(text):
            send(b'\x1b[200~'+text.encode()+b'\x1b[201~')
            time.sleep(.5)
            send(b'\r')
        try:
            wait(lambda text: 'Ask Codex' in text)
            time.sleep(15)
            if attach:
                client = ['env', 'CODEX24H_MAIL=0', 'CODEX24H_PIN_ROWS=6',
                          str(e2e.BIN), 'attach', '--socket', socket, pid]
                pane, supervisor = tmux('new-window', '-d', '-t', 'probe', '-P', '-F', '#{pane_id} #{pane_pid}', shlex.join(client)).split()
                wait(lambda text: 'Ask Codex' in text)
                children = Path(f'/proc/{supervisor}/task/{supervisor}/children').read_text().split()
                pid, = [child for child in children if Path(f'/proc/{child}/exe').resolve().name == 'codex24h']
            enter('不要调用工具。输出40行，每行 PINROW-加三位行号，从001到040，不要省略。')
            wait(lambda text: any(line.strip() == 'PINROW-040' for line in text.splitlines()))
            send(b'draft-one')
            wait(lambda text: 'draft-one' in text)
            send(b'\x1b[5~')
            text = wait(lambda text: '-- live input' in text)
            frozen = text.split('-- live input', 1)[0]
            send('附加中文'.encode())
            text = wait(lambda text: 'draft-one附加中文' in text and '-- live input' in text)
            assert text.split('-- live input', 1)[0] == frozen, 'editing moved historical rows'
            send(b'\x15/mo')
            time.sleep(.6)
            send(b'\t')
            wait(lambda text: '› /model' in text and '-- live input' in text)
            send(b'\x15')
            print('PASS: history stays fixed, Chinese draft and native Tab completion remain live', flush=True)
            send(b'\x1db')
            wait(lambda text: '-- live input' not in text and 'Ask Codex' in text)
            print('PASS: returning to the full native screen', flush=True)
        finally:
            subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)


if __name__ == '__main__':
    main()
