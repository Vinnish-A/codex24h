#!/usr/bin/env python3
"""Opt-in: real local Codex TUI, three turns plus resume, existing login."""
import importlib.machinery
import os
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import e2e

helper = importlib.machinery.SourceFileLoader('requests_helper', str(e2e.ROOT/'scripts/codex24h-requests')).load_module()

def saved_session(path):
    data=helper.read_session(path)
    # Test oracle only; production helper returns request labels, never answers.
    data['entries']=[]
    for raw in path.read_text().splitlines():
        try:r=json.loads(raw)
        except ValueError:continue
        p=r.get('payload',{})
        if r.get('type')=='response_item' and p.get('type')=='message' and p.get('role')=='assistant':
            data['entries'].append('CODEX\n'+''.join(c.get('text','') for c in p.get('content',[])))
    return data

def main():
    with tempfile.TemporaryDirectory(prefix='codex24h-native-requests-') as tmp:
        socket = str(Path(tmp)/'tmux.sock')
        def tmux(*args):
            return subprocess.check_output(['tmux', '-S', socket, *args], text=True).strip()
        def start(args=(), new=False):
            cmd = shlex.join(['env', 'CODEX24H_MAIL=0', str(e2e.BIN), '-c', 'notify=[]', *args])
            opts = ['new-window', '-t', 'probe'] if new else ['-f', '/dev/null', 'new-session', '-s', 'probe', '-x', '100', '-y', '32']
            return tmux(*opts, '-d', '-c', str(e2e.ROOT), '-P', '-F', '#{pane_id} #{pane_pid}', cmd).split()
        pane, pid = start()
        tmux("set-option", "-g", "remain-on-exit", "on")
        def screen(): return tmux('capture-pane', '-p', '-t', pane)
        def send(data): tmux('send-keys', '-t', pane, '-H', *[f'{v:02x}' for v in data])
        def wait(pred, label, timeout=90):
            end = time.monotonic()+timeout
            while time.monotonic()<end:
                if pred(): return
                time.sleep(.15)
            raise AssertionError(label+'\n'+screen())
        home = Path(os.environ.get('CODEX_HOME', str(Path.home()/'.codex')))
        try:
            wait(lambda:'Ask Codex' in screen(), 'native ready')
            prompt = '不要使用工具，只回复两个英文单词连起来：REQUEST 和 OK。'
            for n, text in enumerate([prompt, prompt, '中文长请求。'+('这是用于历史导航测试的文字。'*30)+'不要使用工具，只回复 LONG 和 OK 连起来。'], 1):
                send(b'\x1b[200~'+text.encode()+b'\x1b[201~');time.sleep(.4);send(b'\r')
                path = None
                def saved():
                    nonlocal path
                    try:
                        path = helper.rollout(int(pid),home)
                        data=saved_session(path)
                        return len(data['requests'])>=n and any(e.startswith('CODEX\n') and ('LONGOK' if n==3 else 'REQUESTOK') in e for e in data['entries'][-1:])
                    except RuntimeError:return False
                wait(saved, 'saved native turn '+str(n),180)
                print('PASS native turn',n,flush=True)
                if n == 1:
                    # Check the menu during subsequent live activity below.
                    session=saved_session(path)['session']
            # Start one longer turn, then browse while the model keeps writing.
            send(b'\x1b[200~'+ '不要调用工具。输出80行，每行写STREAMROW加三位行号，从001到080，不要省略。'.encode()+b'\x1b[201~');time.sleep(.4);send(b'\r')
            wait(lambda:len(saved_session(path)['requests']) == 4,'fourth request saved')
            send(b'\x1dr');wait(lambda:'Requests (4)' in screen(),'streaming request list')
            send(b'\x1b[H');time.sleep(.3)
            frozen=screen().split('REQUESTS ·')[0]
            wait(lambda:any('STREAMROW080' in e for e in saved_session(path)['entries'][-1:]), 'background completes',180)
            assert screen().split('REQUESTS ·')[0] == frozen, 'streaming moved the selected request'
            print('PASS background completes while request selection stays frozen',flush=True)
            send(b'\x1b');time.sleep(.2)
            send(b'\x1dr')
            wait(lambda:'Requests (4)' in screen(),'three request list')
            assert screen().count('REQUEST 和 OK') == 2, screen()
            send(b'\x1b[H\r')
            wait(lambda:'native terminal history' in screen() and 'REQUESTOK' in screen(),'first context');assert '› 不要使用工具' in screen() and '• REQUESTOK' in screen() and 'TOOL RESULT' not in screen(),screen()
            (e2e.ROOT/'test-artifacts'/'requests-native-view.txt').write_text(screen())
            send(b'\x1dr');wait(lambda:'Requests (4)' in screen(),'reopen list');send('中文长请求'.encode())
            wait(lambda:'中文长请求' in screen() and '> 3' in screen(),'Chinese filter')
            send(b'\r');wait(lambda:'native terminal history' in screen(),'long context')
            tmux('resize-window','-t','probe','-x','44','-y','16');time.sleep(.5)
            assert 'raw' not in screen() and 'TOOL RESULT' not in screen(),screen()
            send(b'\x1b[6~');time.sleep(.3)
            tmux('resize-window','-t','probe','-x','100','-y','32')
            send(b'\x1db');wait(lambda:'REQUESTS ·' not in screen() and 'SESSION CONTEXT' not in screen(),'return live')
            send(b'draft-kept');time.sleep(.3);send(b'\x1dr');wait(lambda:'Requests (4)' in screen(),'reopen')
            send(b'\x1b');time.sleep(.3);assert 'draft-kept' in screen(),screen()
            send(b'\x15');time.sleep(.2);send(b'\x03');time.sleep(.3);send(b'\x03')
            wait(lambda:tmux('display-message','-p','-t',pane,'#{pane_dead}') == '1','exit own wrapper')
            # Keep the private tmux server alive even if its only pane exited.
            pane,pid=start(('resume',session),new=True)
            wait(lambda:'Ask Codex' in screen(),'resume ready')
            send(b'\x1dr');wait(lambda:'Requests (4)' in screen() and '[unavailable]' not in screen(),'resume old requests')
            send(b'\x1b[H\r');wait(lambda:'REQUESTOK' in screen() and 'native terminal history' in screen(),'resume first context')
            print('PASS repeated requests, Chinese filter, context jump, resize, draft preservation and resume',flush=True)
        finally:
            subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)
if __name__=='__main__':main()
