#!/usr/bin/env python3
"""Browse an existing completed session through ordinary resume; no model requests."""
import argparse
import importlib.machinery
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile
import time
import e2e


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('session', help='UUID of an existing completed TEST session')
    parser.add_argument('--binary', type=Path, default=e2e.BIN)
    parser.add_argument('--via-picker', action='store_true', help='start an empty session, then use /resume')
    parser.add_argument('--switch-to', help='resume another completed TEST session in the same frontend')
    parser.add_argument('--size', default='160x40', help='terminal COLSxROWS')
    args = parser.parse_args()
    cols, rows = args.size.split('x')
    helper = importlib.machinery.SourceFileLoader('requests_helper', str(e2e.ROOT/'scripts/codex24h-requests')).load_module()
    home = Path(os.environ.get('CODEX_HOME', str(Path.home()/'.codex')))
    original = helper.read_shared_session(home, args.session)
    requests = [r['text'] for r in original['requests']]
    assert requests, 'Choose a completed test session with saved requests'
    with tempfile.TemporaryDirectory(prefix='codex24h-native-resume-') as directory:
        socket = str(Path(directory)/'tmux.sock')
        def tmux(*items):
            return subprocess.check_output(['tmux', '-S', socket, *items], text=True).strip()
        command = ['env', 'CODEX24H_MAIL=0', str(args.binary.resolve()), *([] if args.via_picker else ['resume', args.session])]
        pane, pid = tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'probe', '-x', cols, '-y', rows,
                         '-c', str(e2e.ROOT), '-P', '-F', '#{pane_id} #{pane_pid}', shlex.join(command)).split()
        tmux('set-option', '-w', '-t', pane, 'remain-on-exit', 'on')
        def send(data):
            tmux('send-keys', '-t', pane, '-H', *[f'{b:02x}' for b in data])
        def wait(predicate, label, timeout=30):
            deadline = time.monotonic()+timeout
            while time.monotonic() < deadline:
                view = tmux('capture-pane', '-p', '-t', pane)
                if predicate(view):
                    return view
                if 'Update available' in view and 'esc skip' in view.lower():
                    send(b'\x1b')
                if 'Working directory' in view and 'esc use session' in view.lower():
                    send(b'\r')
                time.sleep(.15)
            if os.environ.get('CODEX24H_TEST_CAPTURE'):
                Path(os.environ['CODEX24H_TEST_CAPTURE']).write_text(view)
            raise AssertionError(label+' timed out; footer: '+repr(view.splitlines()[-1]))
        try:
            wait(lambda v: 'Ask Codex' in v and 'Resuming session' not in v and ('for agents' in v or '? for shortcuts' in v), 'shared resume ready')
            for target in [args.session, *([args.switch_to] if args.switch_to else [])]:
                original = helper.read_shared_session(home, target)
                requests = [r['text'] for r in original['requests']]
                if args.via_picker or target != args.session:
                    send(b'/resume'); time.sleep(.2); send(b'\r')
                    wait(lambda v: 'Resume a previous session' in v, 'native resume picker')
                    send(target.encode()); time.sleep(.4); send(b'\r')
                    wait(lambda v: 'Ask Codex' in v and 'Resume a previous session' not in v and 'Resuming session' not in v, 'resume selected session')
                for index, text in enumerate(requests):
                    send(b'\x1dr')
                    view = wait(lambda v: f'Requests ({len(requests)})' in v, 'saved request list')
                    send(b'\x1b[H'+b'\x1b[B'*index+b'\r')
                    query = next((s.strip() for s in text.splitlines() if len(s.strip())>=12), text.strip())[:160]
                    assert query, 'This probe requires requests with text'
                    def found(v):
                        lines = v.splitlines()
                        stop = next((i for i, line in enumerate(lines) if line.lstrip().startswith('Find:') or '-- live input' in line), len(lines)-1)
                        body = ''.join(''.join(lines[:stop]).split())
                        return (''.join(query.split()) in body
                                and ('native terminal history' in v or 'NATIVE HISTORY' in v)
                                and ' · searching' not in v)
                    view = wait(found, 'request '+str(index+1)+' in conversation body')
                    send(b'\x1db')
                    wait(lambda v: 'Ask Codex' in v and 'T R A N S C R I P T' not in v and 'HISTORY (frozen)' not in v, 'return live')
                    print('PASS ordinary shared resume request', index+1, flush=True)
                assert helper.read_shared_session(home, target)==original, 'Navigation submitted or changed requests'
                print(json.dumps({'session': target, 'via_picker': args.via_picker or target != args.session, 'requests_located': len(requests), 'no_model_request': True}), flush=True)
        finally:
            if Path(f'/proc/{pid}').exists():
                os.kill(int(pid), signal.SIGTERM)
            subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)


if __name__ == '__main__':
    main()
