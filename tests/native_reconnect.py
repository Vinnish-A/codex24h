#!/usr/bin/env python3
"""Real shared-backend reconnect probe; default submits one isolated sleep task.
Use --session UUID to reopen an existing TEST session without submitting a turn.
"""
import argparse
import importlib.machinery
import json
import os
from pathlib import Path
import shlex
import shutil
import sqlite3
import subprocess
import tempfile
import time
from unittest.mock import patch
import e2e

helper = importlib.machinery.SourceFileLoader('attach_native', str(e2e.ROOT/'scripts/codex24h-attach')).load_module()
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--session')
parser.add_argument('--codex', default=shutil.which('codex'))
args = parser.parse_args()
home = Path(os.environ.get('CODEX_HOME', str(Path.home()/'.codex')))
with tempfile.TemporaryDirectory(prefix='codex24h-native-reconnect-') as directory:
    base=Path(directory); socket=str(base/'tmux.sock'); events=base/'events'; terminal=None
    def tmux(*items):
        return subprocess.check_output(['tmux','-S',socket,*items],text=True).strip()
    def screen():return tmux('capture-pane','-p','-t','probe')
    def send(data):tmux('send-keys','-t','probe','-H',*[f'{b:02x}' for b in data])
    def wait(predicate, timeout=90):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            if terminal:terminal.drain(.1)
            if predicate():return
            time.sleep(.1)
        raise AssertionError('native reconnect timed out: '+(e2e.visible(terminal.output)[-4000:] if terminal else screen()[-2000:]))
    def identity():
        if args.session:return args.session
        if not events.exists():return None
        for line in events.read_text().splitlines():
            try:record=json.loads(line)
            except ValueError:continue
            client=record.get('payload',{}).get('UserTurn',{}).get('client_user_message_id')
            if client:
                with sqlite3.connect((home/'thread_history_1.sqlite').as_uri()+'?mode=ro',uri=True) as db:
                    row=db.execute("select thread_id from thread_items where item_type='userMessage' and json_extract(item_json,'$.clientId')=?",(client,)).fetchone()
                if row:return row[0]
    def status(session):
        with sqlite3.connect((home/'thread_history_1.sqlite').as_uri()+'?mode=ro',uri=True) as db:
            return db.execute('select status from thread_turns where thread_id=? order by rollout_ordinal desc limit 1',(session,)).fetchone()[0]
    def open_popup(session):
        start=len(terminal.output);terminal.send(b'\x02h');terminal.until(session,since=start)
        env,cwd=helper.native_process(tmux('display-message','-p','-t',pane,'#{pane_tty}'))
        rows=helper.session_rows(home,helper.shared_sessions(home,env))
        rows=[r for r in rows if helper.same_directory(r[2],cwd)] or rows
        index=next(i for i,row in enumerate(rows,1) if row[0]==session)
        start=len(terminal.output);terminal.send(f'{index}\r'.encode())
        terminal.until('return to original page',since=start);terminal.drain(3)
        if 'Skipuntilnextversion' in e2e.visible(terminal.output[start:]).replace(' ', ''):
            terminal.send(b'\x1b')
        wait(lambda:'foragents' in e2e.visible(terminal.output[start:]).replace(' ', ''))
    try:
        command=['env','CODEX24H_MAIL=0','CODEX_TUI_RECORD_SESSION=1',f'CODEX_TUI_SESSION_LOG_PATH={events}',args.codex]
        if args.session:command+=['resume',args.session]
        pane,pid=tmux('-f','/dev/null','new-session','-d','-s','probe','-x','100','-y','32','-c',str(e2e.ROOT),'-P','-F','#{pane_id} #{pane_pid}',shlex.join(command)).split()
        wait(lambda:'Update available' in screen() or 'for agents' in screen())
        if 'Update available' in screen():
            send(b'\x1b');wait(lambda:'for agents' in screen())
        if not args.session:
            prompt='这是独立的前端重连测试。只使用终端运行 sleep 35，完成后只回答 RECONNECT-PROBE-DONE。不要发邮件，不要读写项目文件。'
            send(b'\x1b[200~'+prompt.encode()+b'\x1b[201~');time.sleep(.4);send(b'\r')
            wait(identity);wait(lambda:'sleep 35' in screen() and 'esc to interrupt' in screen())
        session=identity();print('Test session:',session,flush=True)
        send(b'ORIGINAL-DRAFT')
        config=base/'tmux.conf';config.write_text(helper.trigger_config(str(e2e.BIN.resolve())));tmux('source-file',str(config))
        env=os.environ|{'TERM':'xterm-256color','CODEX24H_MAIL':'0'};env.pop('TMUX',None)
        with patch.object(e2e,'BIN',Path('/usr/bin/tmux')):
            terminal=e2e.Terminal(env,['-S',socket,'attach-session','-t','probe'])
        terminal.drain(.4);open_popup(session)
        assert 'ORIGINAL-DRAFT' in screen()
        terminal.send(b'\x1dd');terminal.drain(1)
        if not args.session:
            assert status(session)=='inProgress', 'task ended before detach continuity could be verified'
            wait(lambda:status(session)=='completed',150)
        os.kill(int(pid),0)
        terminal.send(b'-RETURNED');wait(lambda:'ORIGINAL-DRAFT-RETURNED' in screen())
        print('PASS original draft/process preserved and frontend detach'+(' during a running backend task' if not args.session else ''),flush=True)
        open_popup(session)
        start=len(terminal.output);terminal.send(b'\x1dr');terminal.until('REQUESTS ·',since=start)
        assert 'session record is not available' not in e2e.visible(terminal.output[start:])
        start=len(terminal.output);terminal.send(b'\r');wait(lambda: any(marker in e2e.visible(terminal.output[start:]) for marker in ('native terminal history', 'NATIVE HISTORY')), 20)
        terminal.send(b'\x1b[5~');terminal.drain(.2)
        if 'native terminal history' in e2e.visible(terminal.output[start:]):
            assert 'liveinput' in e2e.visible(terminal.output).replace(' ','')
        terminal.send(b'\x03');terminal.drain(.6)
        # Native quit exits the new frontend, then popup returns to original composer.
        terminal.send(b'/quit');terminal.drain(.3);terminal.send(b'\r');terminal.drain(2)
        terminal.send(b'-AFTER-QUIT');wait(lambda:'ORIGINAL-DRAFT-RETURNED-AFTER-QUIT' in screen())
        print('PASS request list/jump, scroll with live input, agents affordance and native quit',flush=True)
    finally:
        if terminal:terminal.close()
        subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)
