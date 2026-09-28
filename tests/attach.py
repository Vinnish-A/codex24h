#!/usr/bin/env python3
"""Local tmux attach integration. --native also sends one real Codex task."""
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import unittest

import e2e


class AttachTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='codex24h-attach-test-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.socket = str(self.base/'tmux.sock')
        self.env = os.environ | {'TERM': 'xterm-256color', 'CODEX24H_MAIL': '0'}
        command = ['env', f'FAKE_LOG_DIR={self.base}', str(e2e.FAKE)]
        self.pane, pid = self.tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'original',
                                  '-x', '80', '-y', '23', '-P', '-F', '#{pane_id} #{pane_pid}',
                                  shlex.join(command)).split()
        self.pid = int(pid)
        self.addCleanup(self.stop_server)
        self.wait(lambda: (self.base/'pid.log').exists())

    def tmux(self, *args):
        return subprocess.check_output(['tmux', '-S', self.socket, *args], text=True).strip()

    def stop_server(self):
        subprocess.run(['tmux', '-S', self.socket, 'kill-server'], capture_output=True)

    def wait(self, predicate, terminal=None, timeout=8):
        end = time.monotonic()+timeout
        while time.monotonic()<end:
            if terminal:
                terminal.drain(.08)
            if predicate():
                return
            time.sleep(.02)
        self.fail('attach test condition timed out')

    def attach(self, pid=None):
        terminal = e2e.Terminal(self.env, ['attach', '--socket', self.socket, str(pid or self.pid)])
        self.addCleanup(terminal.close)
        return terminal

    def body(self, terminal):
        return e2e.screen_text(terminal.output)

    def log(self, name):
        path = self.base/name
        return path.read_bytes() if path.exists() else b''

    def test_scroll_question_detach_and_reattach_keep_pid(self):
        terminal = self.attach()
        self.wait(lambda: 'READY-080' in '\n'.join(self.body(terminal)), terminal)
        terminal.send(b':burst 120\n')
        self.wait(lambda: 'NEW-120' in '\n'.join(self.body(terminal)), terminal)
        terminal.send(b'\x1b[<64;10;10M'*5)
        self.wait(lambda: 'new updates' in self.body(terminal)[-1], terminal)
        frozen = self.body(terminal)[:-1]
        terminal.send(b':burst 10\n')
        self.wait(lambda: b'burst:10' in self.log('events.log'), terminal)
        terminal.drain(.4)
        self.assertEqual(self.body(terminal)[:-1], frozen)
        terminal.send(b'\x1db:question\n')
        self.wait(lambda: 'QUESTION:Choose a review path' in self.body(terminal)[0], terminal)
        terminal.send(b'\x1b[B\r')
        self.wait(lambda: b'option:Review changes' in self.log('answer.log'), terminal)
        terminal.resize(31, 92)
        self.wait(lambda: b'30x92' in self.log('size.log'), terminal)
        terminal.send(b'\x1dd')
        self.assertEqual(terminal.wait_draining(), 0)
        terminal.assert_restored(self)
        os.kill(self.pid, 0)
        self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')
        self.assertEqual(self.tmux('show-options', '-Av', '-t', 'original', 'status'), 'on')
        self.assertEqual(self.tmux('show-options', '-Av', '-t', 'original', 'prefix'), 'C-b')
        again = self.attach()
        self.wait(lambda: 'ANSWER:option:Review changes' in '\n'.join(self.body(again)), again)
        again.send(b':burst 1\n')
        self.wait(lambda: b'burst:1\n' in self.log('events.log'), again)
        again.send(b'\x1dd')
        self.assertEqual(again.wait_draining(), 0)
        self.assertEqual(self.log('pid.log').splitlines(), [str(self.pid).encode()])

    def test_external_signal_only_disconnects_client(self):
        terminal = self.attach()
        self.wait(lambda: 'attached' in self.body(terminal)[-1], terminal)
        os.kill(terminal.process.pid, signal.SIGTERM)
        self.assertEqual(terminal.wait_draining(), 143)
        terminal.assert_restored(self)
        os.kill(self.pid, 0)
        self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')

    def test_removing_original_session_does_not_kill_target_on_detach(self):
        terminal = self.attach()
        self.wait(lambda: 'READY-080' in '\n'.join(self.body(terminal)), terminal)
        self.tmux('kill-session', '-t', 'original')
        terminal.send(b'\x1dd')
        self.assertEqual(terminal.wait_draining(), 0)
        os.kill(self.pid, 0)
        self.assertIn('codex24h-attach-', self.tmux('list-sessions', '-F', '#{session_name}'))

    def test_unsupported_plain_process_is_untouched(self):
        native = e2e.BIN
        try:
            e2e.BIN = e2e.FAKE
            target = e2e.Terminal(self.env | {'FAKE_LOG_DIR': str(self.base)})
        finally:
            e2e.BIN = native
        self.addCleanup(target.close)
        target.until('READY-080')
        terminal = self.attach(target.process.pid)
        self.assertEqual(terminal.wait_draining(), 1)
        self.assertIn('目标不在此 tmux server', e2e.visible(terminal.output))
        self.assertIsNone(target.process.poll())
        target.send(b':burst 1\n')
        target.until('NEW-001')
        terminal.assert_restored(self)


def native_probe():
    with tempfile.TemporaryDirectory(prefix='codex24h-native-attach-') as directory:
        socket = str(Path(directory)/'tmux.sock')
        def tmux(*args):
            return subprocess.check_output(['tmux', '-S', socket, *args], text=True).strip()
        command = ['codex', '-c', 'notify=[]']
        pane, pid = tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'native', '-x', '100',
                         '-y', '30', '-c', str(e2e.ROOT), '-P', '-F', '#{pane_id} #{pane_pid}',
                         shlex.join(command)).split()
        terminal = None
        def screen():
            return tmux('capture-pane', '-p', '-t', pane)
        def send(data):
            tmux('send-keys', '-t', pane, '-H', *[f'{byte:02x}' for byte in data])
        def wait(predicate, timeout=60):
            end = time.monotonic()+timeout
            while time.monotonic()<end:
                if terminal:
                    terminal.drain(.1)
                if predicate():
                    return
                time.sleep(.1)
            raise AssertionError('native attach timed out; test screen: '+screen()[-1800:])
        try:
            wait(lambda: 'Ask Codex' in screen())
            time.sleep(1)
            prompt = '请使用终端执行 sleep 12，结束后只回答 731 加 29 的结果。不要读写文件。'
            send(prompt.encode()); time.sleep(.3); send(b'\r')
            wait(lambda: 'Working' in screen() or 'sleep 12' in screen() and 'esc to interrupt' in screen())
            start = Path(f'/proc/{pid}/stat').read_text().split(') ', 1)[1].split()[19]
            terminal = e2e.Terminal(os.environ | {'TERM':'xterm-256color','CODEX24H_MAIL':'0'},
                                    ['attach', '--socket', socket, pid])
            wait(lambda: 'attached' in e2e.screen_text(terminal.output)[-1])
            print('PASS: attached while native Codex task was in progress', flush=True)
            wait(lambda: '760' in screen() and 'esc to interrupt' not in screen(), timeout=120)
            assert Path(f'/proc/{pid}/stat').read_text().split(') ', 1)[1].split()[19] == start
            print('PASS: same Codex PID/start time completed the pre-attach task', flush=True)
            terminal.send(b'\x1b[A')
            wait(lambda: '› '+prompt[:12] in screen())
            terminal.send(b'\x1b[B'); time.sleep(.3)
            terminal.send(b'/mo'); time.sleep(.5); terminal.send(b'\t')
            wait(lambda: '› /model' in screen())
            terminal.send(b'\r')
            wait(lambda: 'Select Model' in screen() or 'Select model' in screen())
            terminal.send(b'\x1b')
            wait(lambda: 'Select Model' not in screen() and 'Select model' not in screen())
            print('PASS: native input history, Tab completion and model menu', flush=True)
            terminal.send(b'\x1dd')
            assert terminal.wait_draining() == 0
            os.kill(int(pid), 0)
            assert tmux('list-sessions', '-F', '#{session_name}') == 'native'
            send(b'/mo'); time.sleep(.3); send(b'\t')
            wait(lambda: '› /model' in screen())
            send(b'\x15')
            print('PASS: detach leaves the original terminal and Codex usable', flush=True)
        finally:
            if terminal:
                terminal.close()
            subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)


if __name__ == '__main__':
    if sys.argv[1:] == ['--native']:
        native_probe()
    else:
        unittest.main(verbosity=2)
