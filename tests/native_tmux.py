#!/usr/bin/env python3
"""Exercise an existing long conversation through a detached local tmux pane.

Uses existing Codex credentials. Sends only wheel, editing and completion keys;
never submits a model prompt. Requires an existing tmux server.
"""
import argparse
import json
import os
import signal
import subprocess
import time


def tmux(*args):
    return subprocess.check_output(['tmux', *args], text=True)


def run(args):
    command = [args.binary, 'resume', args.session]
    import shlex
    pane, pid = tmux('new-window', '-d', '-P', '-F', '#{pane_id} #{pane_pid}',
                     '-n', 'codex24h-long-test', '-c', args.cwd,
                     shlex.join(command)).split()
    pid = int(pid)

    def screen():
        return tmux('capture-pane', '-p', '-t', pane).splitlines()

    def send(data):
        tmux('send-keys', '-t', pane, '-H', *[f'{b:02x}' for b in data])
        time.sleep(.15)

    def wait_for(predicate, label, timeout=45):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            lines = screen()
            if predicate(lines):
                return lines
            time.sleep(.1)
        raise AssertionError(f'timed out: {label}')

    def composer(lines):
        return next((line.strip() for line in reversed(lines) if line.startswith('› ')), '')

    started = time.monotonic()
    try:
        lines = wait_for(lambda s: composer(s) == '› Ask Codex to do anything', 'composer ready')
        print(f'Composer visible after {time.monotonic() - started:.2f}s', flush=True)
        # Let transcript hydration finish before navigating it.
        time.sleep(2)
        seen = set()
        for _ in range(40):
            send(b'\x1b[<64;20;10M')
            body = tuple(screen()[:-1])
            seen.add(body)
        assert len(seen) > 10, f'insufficient scrollable history: {len(seen)} distinct views'
        frozen = screen()[:-1]
        time.sleep(1.2)
        assert screen()[:-1] == frozen, 'background output changed the frozen body'
        assert 'new updates' in screen()[-1]
        print(f'Wheel up: {len(seen)} distinct historical views; frozen body stable', flush=True)
        for _ in range(20):
            send(b'\x1b[<65;20;10M' * 25)
            if screen()[-1].startswith('codex24h ·'):
                break
        wait_for(lambda s: s[-1].startswith('codex24h ·'), 'wheel down restores follow')
        print('Wheel down restored follow mode', flush=True)

        lines = screen()
        row = max(i + 1 for i, line in enumerate(lines) if line.startswith('› '))
        send(f'\x1b[<0;3;{row}M\x1b[<0;3;{row}m'.encode())
        assert not screen()[-1].startswith('COPY'), 'ordinary composer click entered Copy mode'
        before = composer(screen())
        send(b'\x1b[A')
        wait_for(lambda s: composer(s) not in ('', before), 'Up recalls native input history')
        send(b'\x1b[B')
        wait_for(lambda s: composer(s) == before, 'Down restores draft')
        print('Composer click + Up/Down: native input history works', flush=True)

        send(b'/mo')
        wait_for(lambda s: any('/model' in line and 'choose' in line for line in s), 'native completion menu')
        send(b'\t')
        wait_for(lambda s: composer(s) == '› /model', 'Tab completes /model')
        send(b'\x15')
        print('Native /mo menu + Tab completion works; draft cleared without submitting', flush=True)
        send(b'\x1d[')
        assert screen()[-1].startswith('COPY'), 'explicit Copy mode unavailable'
        send(b'\x1db')
        print(json.dumps({'result': 'PASS', 'historical_views': len(seen), 'elapsed_seconds': round(time.monotonic()-started, 2)}), flush=True)
    finally:
        # Disconnect only this test client; do not interrupt a resumed active turn.
        if os.path.exists(f'/proc/{pid}'):
            os.kill(pid, signal.SIGTERM)
            end = time.monotonic() + 4
            while os.path.exists(f'/proc/{pid}') and time.monotonic() < end:
                time.sleep(.05)
            if os.path.exists(f'/proc/{pid}'):
                tmux('kill-pane', '-t', pane)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('session', help='existing long Codex conversation ID')
    parser.add_argument('--cwd', default=os.getcwd())
    parser.add_argument('--binary', default=os.path.expanduser('~/.local/bin/codex24h'))
    run(parser.parse_args())
