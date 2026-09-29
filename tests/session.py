#!/usr/bin/env python3
"""Real kernel-lock and process-control tests; no user Codex is signalled."""
import importlib.machinery
import os
from pathlib import Path
import shutil
import shlex
import e2e
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
helper = importlib.machinery.SourceFileLoader('session_helper', str(ROOT/'scripts/codex24h-session')).load_module()


class SessionTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='codex24h-session-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root/'11111111-1111-4111-8111-111111111111.lock'
        self.exe = self.root/'codex'
        shutil.copyfile(sys.executable, self.exe)
        self.exe.chmod(0o700)

    def holder(self, extra=(), lock=True):
        code = '''import fcntl, pathlib, sys, time
files=[]
for name in sys.argv[1:]:
 if name == 'app-server': continue
 f=open(name,'w'); files.append(f)
 if SHOULD_LOCK: fcntl.flock(f,fcntl.LOCK_EX)
print('READY',flush=True)
time.sleep(30)
'''.replace('SHOULD_LOCK', repr(lock))
        p = subprocess.Popen([str(self.exe), '-c', code, str(self.path), *map(str,extra)],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             env=os.environ | {'PYTHONHOME':sys.base_prefix})
        def close():
            if p.poll() is None: p.terminate()
            p.wait(timeout=3)
            p.stdout.close(); p.stderr.close()
        self.addCleanup(close)
        self.assertEqual(p.stdout.readline(), b'READY\n')
        return p

    def test_stale_file_and_open_descriptor_are_not_owners(self):
        self.path.touch()
        self.assertEqual(helper.owners(self.path), [])
        self.holder(lock=False)
        self.assertEqual(helper.owners(self.path), [])

    def test_stop_only_matching_single_writer_and_release_lock(self):
        p = self.holder()
        self.assertEqual(helper.owners(self.path), [p.pid])
        helper.stop_tui(p.pid, self.path)
        p.wait(timeout=3)
        self.assertEqual(helper.owners(self.path), [])
        self.assertTrue(self.path.exists(), 'must never unlink a native lock')

    def test_shared_app_server_is_untouched(self):
        p = self.holder(['app-server'])
        with self.assertRaisesRegex(RuntimeError, 'app-server'):
            helper.stop_tui(p.pid, self.path)
        self.assertIsNone(p.poll())

    def test_multiple_sessions_are_untouched(self):
        p = self.holder([self.root/'22222222-2222-4222-8222-222222222222.lock'])
        with self.assertRaisesRegex(RuntimeError, '多个会话'):
            helper.stop_tui(p.pid, self.path)
        self.assertIsNone(p.poll())

    def test_wrong_session_is_untouched(self):
        p = self.holder()
        with self.assertRaisesRegex(RuntimeError, '持锁状态'):
            helper.stop_tui(p.pid, self.root/'other.lock')
        self.assertIsNone(p.poll())

    def test_attach_returns_to_existing_tmux_writer(self):
        socket = str(self.root/'tmux.sock')
        home = self.root/'home'
        lockdir = home/'thread-writer-locks'
        lockdir.mkdir(parents=True)
        path = lockdir/self.path.name
        code = "import fcntl,sys,time; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); print('SESSION_OWNER_READY',flush=True); time.sleep(30)"
        command = ['env', 'PYTHONHOME='+sys.base_prefix, str(self.exe), '-c', code, str(path)]
        subprocess.check_call(['tmux','-S',socket,'-f','/dev/null','new-session','-d','-s','owner',shlex.join(command)])
        self.addCleanup(lambda:subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True))
        deadline=time.monotonic()+5
        while not helper.owners(path) and time.monotonic()<deadline:time.sleep(.02)
        original=helper.owners(path)
        self.assertEqual(len(original),1)
        terminal=e2e.Terminal(os.environ|{'TERM':'xterm-256color','CODEX_HOME':str(home),'CODEX24H_MAIL':'0'},
                             ['session',path.stem,'--attach','--socket',socket])
        self.addCleanup(terminal.close)
        terminal.until('SESSION_OWNER_READY')
        terminal.send(b'\x02d')
        self.assertEqual(terminal.wait_draining(),0)
        self.assertEqual(helper.owners(path),original)

    def test_uuid_cannot_escape_codex_home(self):
        with self.assertRaises(ValueError): helper.lock_path('../../etc/passwd')


if __name__ == '__main__': unittest.main()
