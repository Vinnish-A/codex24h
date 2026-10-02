#!/usr/bin/env python3
"""Reconnect selection and real tmux popup lifecycle (fake Codex, no model calls)."""
import contextlib
import importlib.machinery
import io
import json
import os
from pathlib import Path
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import e2e

helper = importlib.machinery.SourceFileLoader('attach_helper', str(e2e.ROOT/'scripts/codex24h-attach')).load_module()
SESSION = '00000000-0000-4000-8000-000000000001'


class SelectionTest(unittest.TestCase):
    def test_state_filters_children_and_unowned_sessions(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            with sqlite3.connect(home/'state_5.sqlite') as db:
                db.execute('create table threads(id, name, title, cwd, source, archived, updated_at)')
                db.execute('create table thread_spawn_edges(child_thread_id)')
                db.executemany('insert into threads values(?,?,?,?,?,?,?)', [
                    ('root','named','title',directory,'cli',0,1),
                    ('child',None,'child',directory,'cli',0,2),
                    ('other',None,'other',directory,'cli',0,3),
                    ('sub',None,'sub',directory,'subagent',0,4)])
                db.execute("insert into thread_spawn_edges values('child')")
            self.assertEqual(helper.session_rows(home, {'root','child','sub'}), [('root','named',directory)])
            with patch('builtins.input', side_effect=['q']), contextlib.redirect_stdout(io.StringIO()):
                self.assertIsNone(helper.choose_session([('root','named',directory)], directory))
            with patch('builtins.input', side_effect=['invalid','1']), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(helper.choose_session([('root','named',directory)], directory)[0], 'root')

    def test_shell_is_not_a_codex_frontend(self):
        with self.assertRaisesRegex(RuntimeError, '没有唯一'):
            helper.native_process('/dev/not-a-codex-tty')

    def test_old_pid_mirror_entry_is_removed(self):
        result = subprocess.run([sys.executable, str(e2e.ROOT/'scripts/codex24h-attach'), '--wrapper', str(e2e.BIN), '1234'],capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'unrecognized arguments', result.stderr)


class PopupTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='codex24h-popup-test-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.original = self.base/'original'; self.original.mkdir()
        self.frontend = self.base/'frontend'; self.frontend.mkdir()
        self.socket = str(self.base/'tmux.sock')
        self.env = os.environ | {'TERM':'xterm-256color','CODEX24H_MAIL':'0'}
        self.env.pop('TMUX',None)
        command = ['env',f'FAKE_LOG_DIR={self.original}',str(e2e.FAKE)]
        self.pane, self.pid = self.tmux('-f','/dev/null','new-session','-d','-s','original','-x','100','-y','30','-P','-F','#{pane_id} #{pane_pid}',shlex.join(command)).split()
        self.addCleanup(lambda: subprocess.run(['tmux','-S',self.socket,'kill-server'],capture_output=True))
        # Substitute only session discovery; use production helper, popup, wrapper and PTY lifecycle.
        self.wrapper = self.base/'wrapper space "$HOME #.bin'
        self.wrapper.write_text(f'''#!{sys.executable}
import importlib.machinery, os, sys
from pathlib import Path
if sys.argv[1:2] == ['attach']:
 m=importlib.machinery.SourceFileLoader('attach_fixture',{str(e2e.ROOT/'scripts/codex24h-attach')!r}).load_module()
 m.native_process=lambda tty:(dict(os.environ,CODEX24H_CODEX={str(e2e.FAKE)!r},CODEX24H_MAIL='0',CODEX24H_RECONNECTED='1',FAKE_LOG_DIR={str(self.frontend)!r}),Path({str(self.base)!r}))
 m.shared_sessions=lambda *args:{{{SESSION!r}}}
 m.session_rows=lambda *args:[({SESSION!r},'TEST-SESSION',{str(self.base)!r})]
 sys.argv=[sys.argv[0],*sys.argv[2:],'--wrapper',sys.argv[0]]
 sys.exit(m.main())
os.execve({str(e2e.BIN.resolve())!r},[{str(e2e.BIN.resolve())!r},*sys.argv[1:]],os.environ)
''')
        self.wrapper.chmod(0o755)
        config = self.base/'trigger.conf'; config.write_text(helper.trigger_config(str(self.wrapper)))
        self.tmux('source-file',str(config))
        with patch.object(e2e,'BIN',Path('/usr/bin/tmux')):
            self.terminal=e2e.Terminal(self.env,['-S',self.socket,'attach-session','-t','original'])
        self.addCleanup(self.terminal.close)
        self.terminal.until('READY-080')

    def tmux(self,*args):
        return subprocess.check_output(['tmux','-S',self.socket,*args],text=True).strip()

    def log(self,folder,name):
        path=folder/name
        return path.read_bytes() if path.exists() else b''

    def wait(self,predicate):
        end=time.monotonic()+8
        while time.monotonic()<end:
            self.terminal.drain(.08)
            if predicate():return
        self.fail('popup condition timed out')

    def open(self):
        start=len(self.terminal.output)
        self.terminal.send(b'\x02h');self.terminal.until('TEST-SESSION',since=start)
        self.terminal.send(b'1\r');self.terminal.until('return to original page',since=start)
        self.terminal.until('READY-080',since=start)

    def test_shortcut_new_frontend_keeps_original_draft_and_exit_returns(self):
        self.terminal.send(b':heartbeat 100\nORIGINAL-DRAFT')
        self.terminal.drain(.2)
        before=self.log(self.original,'stdin.bin')
        self.open()
        new_pid=int(self.log(self.frontend,'pid.log').splitlines()[-1])
        self.assertNotEqual(new_pid,int(self.pid))
        self.terminal.send(b'NEW-DRAFT')
        self.wait(lambda:b'NEW-DRAFT' in self.log(self.frontend,'stdin.bin'))
        self.assertEqual(self.log(self.original,'stdin.bin'),before)
        self.terminal.send(b'\x1dd')
        self.wait(lambda:not Path(f'/proc/{new_pid}').exists())
        self.terminal.send(b'-RETURNED')
        self.wait(lambda:self.log(self.original,'stdin.bin').endswith(b'-RETURNED'))
        self.open()
        self.terminal.send(b'\x1b[5~');self.terminal.drain(.2)
        self.terminal.send(b':exit\n')
        self.wait(lambda:b'exit\n' in self.log(self.frontend,'events.log'))
        self.terminal.drain(.4)
        self.terminal.send(b'-AFTER-EXIT')
        self.wait(lambda:self.log(self.original,'stdin.bin').endswith(b'-AFTER-EXIT'))
        self.assertEqual(self.tmux('list-sessions','-F','#{session_name}'),'original')
        self.assertNotIn(b'\x03',self.log(self.original,'stdin.bin'))
        self.assertGreater(self.log(self.original,'events.log').count(b'beat:'),1)

    def test_duplicate_trigger_and_client_disconnect(self):
        self.open()
        new_pid=int(self.log(self.frontend,'pid.log').splitlines()[-1])
        client=self.tmux('list-clients','-F','#{client_tty}')
        result=subprocess.run([str(self.wrapper),'attach','--popup','--socket',self.socket,'--client',client,'--pane',self.pane],capture_output=True,timeout=3)
        self.assertEqual(result.returncode,0,result.stderr)
        self.terminal.process.terminate();self.terminal.wait_draining()
        self.wait(lambda:not Path(f'/proc/{new_pid}').exists())
        os.kill(int(self.pid),0)
        self.assertEqual(self.tmux('list-sessions','-F','#{session_name}'),'original')

    def test_stale_client_rejected_and_split_window_supported(self):
        client=self.tmux('list-clients','-F','#{client_tty}')
        result=subprocess.run([str(self.wrapper),'attach','--popup','--socket',self.socket,'--client',client,'--pane','%99999'],capture_output=True,timeout=3)
        self.assertNotEqual(result.returncode,0)
        self.tmux('split-window','-d','-t',self.pane,'sleep 60')
        self.open();self.terminal.send(b'\x1dd');self.terminal.drain(.4)
        self.assertEqual(self.tmux('display-message','-p','-t',self.pane,'#{window_panes}'),'2')


if __name__=='__main__':
    unittest.main()
