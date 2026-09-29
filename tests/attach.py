#!/usr/bin/env python3
"""Local tmux attach integration. --native also sends one real Codex task."""
import os
import base64
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
        self.env = os.environ | {'TERM': 'xterm-256color', 'CODEX24H_MAIL': '0', 'CODEX24H_PIN_ROWS': '0'}
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

    def ready(self, terminal):
        self.wait(lambda: 'READY-080' in '\n'.join(self.body(terminal)), terminal)

    def test_modified_keys_bypass_tmux_bindings_and_protocol_reencoding(self):
        # A custom root binding and disabled extended-keys must not take over
        # Codex's question navigation. Keep both settings unchanged afterwards.
        self.tmux('bind-key', '-n', 'S-Left', 'display-message', 'INTERCEPTED')
        terminal = self.attach()
        self.ready(terminal)
        keys = (b'\x1b[1;2D', b'\x1b[1;2C', b'\x1b[Z', b'\x1b[1;5D',
                b'\x1b[13;2u', b'\x1b[27;2;13~', b'\x1b[57350;2u',
                b'\x1b[57351;2u', b'\x1b[97;5u', b'\x1b\r', b'\x1b[5;2~', b'\x1b[6;5~')
        for key in keys:
            with self.subTest(key=key):
                before = len(self.log('stdin.bin'))
                # Fragment inside the CSI token as can happen over SSH.
                terminal.send(key[:2]); terminal.drain(.025); terminal.send(key[2:])
                self.wait(lambda: len(self.log('stdin.bin')) >= before+len(key), terminal)
                self.assertEqual(self.log('stdin.bin')[before:], key)
        self.assertEqual(self.tmux('show-options', '-sv', 'extended-keys'), 'off')
        self.assertIn('INTERCEPTED', self.tmux('list-keys', '-T', 'root', 'S-Left'))

    def test_large_literal_paste_does_not_overflow_control_commands(self):
        terminal = self.attach()
        self.ready(terminal)
        before = len(self.log('stdin.bin'))
        payload = b'\x1b[200~'+('中文🙂 "\\;\n'*1200).encode()+b'\x1dd\x1b[201~'
        for start in range(0,len(payload),1000):
            terminal.send(payload[start:start+1000]); terminal.drain(.005)
        self.wait(lambda: len(self.log('stdin.bin')) >= before+len(payload), terminal)
        self.assertEqual(self.log('stdin.bin')[before:], payload)
        self.assertIsNone(terminal.process.poll())

    def send_source(self, data):
        self.tmux('send-keys', '-t', self.pane, '-H', *[f'{b:02x}' for b in data])

    def test_repeated_attach_has_no_session_or_client_leaks(self):
        elapsed = []
        for _ in range(8):
            started = time.monotonic()
            terminal = self.attach()
            self.ready(terminal)
            elapsed.append(time.monotonic()-started)
            terminal.send(b'\x1dd')
            self.assertEqual(terminal.wait_draining(), 0)
            self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')
            self.assertEqual(self.tmux('list-clients'), '')
            os.kill(self.pid, 0)
        print(f'\n  Eight attaches: {min(elapsed):.2f}–{max(elapsed):.2f}s to visible target', flush=True)

    def test_search_copy_and_paste_remain_separate_from_native_input(self):
        terminal = self.attach()
        self.ready(terminal)
        before = self.log('stdin.bin')
        terminal.send(b'\x1d/ROW-070\r')
        self.wait(lambda: 'COPY' in self.body(terminal)[-1], terminal)
        terminal.send(b'vlllllly')
        copied = b'ROW-070'
        self.wait(lambda: b'\x1b]52;c;'+base64.b64encode(copied) in terminal.output, terminal)
        self.assertIn(b'\x1b]52;c;'+base64.b64encode(copied), terminal.output)
        self.assertEqual(self.log('stdin.bin'), before)
        terminal.send(b'\x1db')
        paste = b'\x1b[200~'+ '中文🙂\n'.encode()+b'\x1dd\x1b[201~'
        for chunk in (paste[:9], paste[9:13], paste[13:]):
            terminal.send(chunk)
            terminal.drain(.12)
        self.wait(lambda: paste in self.log('stdin.bin'), terminal)
        self.assertIsNone(terminal.process.poll(), 'pasted detach chord detached the wrapper')
        terminal.send(b'\x1dd')
        self.assertEqual(terminal.wait_draining(), 0)

    def test_two_clients_freeze_independently_and_keep_original_selection(self):
        other = self.tmux('new-window', '-P', '-F', '#{window_id}', '-t', 'original', 'sleep 120')
        first = self.attach()
        self.ready(first)
        second = self.attach()
        self.ready(second)
        self.assertEqual(self.tmux('display-message', '-p', '-t', 'original', '#{window_id}'), other)
        first.send(b'\x1b[5~'); first.drain(.2)
        frozen = self.body(first)[:-1]
        self.send_source(b':burst 50\n')
        self.wait(lambda: 'NEW-050' in '\n'.join(self.body(second)), second)
        first.drain(.3)
        self.assertEqual(self.body(first)[:-1], frozen)
        first.send(b'\x1dd')
        self.assertEqual(first.wait_draining(), 0)
        second.send(b':burst 51\n')
        self.wait(lambda: b'burst:51' in self.log('events.log'), second)
        second.send(b'\x1dd')
        self.assertEqual(second.wait_draining(), 0)
        self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')

    def test_terminal_hangup_preserves_task_and_cleans_attachment(self):
        terminal = self.attach()
        self.ready(terminal)
        # Simulate closing an SSH terminal: close its master, without asking
        # the wrapper to exit. Replace the descriptor for fixture cleanup only.
        os.close(terminal.master)
        terminal.master = os.open('/dev/null', os.O_RDONLY)
        self.assertIsNotNone(terminal.process.wait(timeout=8))
        os.kill(self.pid, 0)
        self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')
        self.send_source(b':burst 2\n')
        self.wait(lambda: b'burst:2' in self.log('events.log'))

    def test_simultaneous_detach_without_original_keeps_last_window_owner(self):
        for _ in range(4):
            source = self.tmux('list-sessions', '-F', '#{session_id}')
            first, second = self.attach(), self.attach()
            self.ready(first); self.ready(second)
            self.tmux('kill-session', '-t', source)
            first.send(b'\x1dd'); second.send(b'\x1dd')
            self.assertEqual(first.wait_draining(), 0)
            self.assertEqual(second.wait_draining(), 0)
            os.kill(self.pid, 0)
            self.assertEqual(len(self.tmux('list-sessions', '-F', '#{session_id}').splitlines()), 1)

    def test_small_screen_resize_and_flood_do_not_interrupt_target(self):
        terminal = self.attach()
        self.ready(terminal)
        for rows, cols in ((0, 0), (1, 80), (24, 1)):
            terminal.resize(rows, cols)
            terminal.drain(.1)
            self.assertIsNone(terminal.process.poll())
            os.kill(self.pid, 0)

        for rows, cols in ((18, 40), (45, 120), (24, 80)):
            terminal.resize(rows, cols)
            self.wait(lambda: f'{rows-1}x{cols}\n'.encode() in self.log('size.log'), terminal)
        terminal.send(b'\x1b[5~'); terminal.drain(.2)
        frozen = self.body(terminal)[:-1]
        start = time.monotonic()
        self.send_source(b':flood 8\n')
        # No outer terminal reads while tmux and Codex24h consume the flood.
        self.wait(lambda: b'flood:8:done' in self.log('events.log'), timeout=15)
        terminal.drain(.4)
        self.assertEqual(self.body(terminal)[:-1], frozen)
        terminal.send(b'\x1db')
        self.wait(lambda: 'FLOOD-' in '\n'.join(self.body(terminal)), terminal)
        print(f'\n  Attached 8 MiB flood: {time.monotonic()-start:.2f}s, frozen viewport preserved', flush=True)

    def test_copy_mode_and_split_window_rejection_leave_target_untouched(self):
        self.tmux('copy-mode', '-t', self.pane)
        terminal = self.attach()
        self.assertEqual(terminal.wait_draining(), 1)
        self.assertIn('复制/选择模式', e2e.visible(terminal.output))
        self.tmux('send-keys', '-t', self.pane, '-X', 'cancel')
        self.tmux('split-window', '-d', '-t', self.pane, 'sleep 120')
        terminal = self.attach()
        self.assertEqual(terminal.wait_draining(), 1)
        self.assertIn('单窗格窗口', e2e.visible(terminal.output))
        os.kill(self.pid, 0)
        self.assertEqual(self.tmux('list-sessions', '-F', '#{session_name}'), 'original')

    def test_full_screen_limitation_is_visible_in_list_and_footer(self):
        self.send_source(b':alternate\n')
        self.wait(lambda: self.tmux('display-message', '-p', '-t', self.pane, '#{alternate_on}') == '1')
        terminal = self.attach()
        self.wait(lambda: 'ALT-READY' in '\n'.join(self.body(terminal)), terminal)
        self.assertIn('history limited', self.body(terminal)[-1])
        listing = subprocess.check_output([str(e2e.BIN), 'attach', '--socket', self.socket, '--list'], text=True)
        self.assertEqual(listing.count(self.pane+'\t'), 1, 'grouped sessions duplicated a pane in --list')
        self.assertIn('full-screen (history limited)', listing)
        terminal.send(b'\x1dd')
        self.assertEqual(terminal.wait_draining(), 0)

    def test_target_exit_during_browse_keeps_history_until_detach(self):
        terminal = self.attach()
        self.ready(terminal)
        terminal.send(b'\x1b[5~'); terminal.drain(.2)
        self.send_source(b':exit 0\n')
        self.wait(lambda: terminal.process.poll() is not None or 'exited' in self.body(terminal)[-1], terminal)
        self.assertIsNone(terminal.process.poll())
        terminal.send(b'\x1dd')
        terminal.wait_draining()
        terminal.assert_restored(self)

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


def native_probe(inline=False):
    with tempfile.TemporaryDirectory(prefix='codex24h-native-attach-') as directory:
        socket = str(Path(directory)/'tmux.sock')
        def tmux(*args):
            return subprocess.check_output(['tmux', '-S', socket, *args], text=True).strip()
        command = ['codex', '-c', 'notify=[]']
        if inline:
            command.append('--no-alt-screen')
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
            draft = '附着前的草稿🙂'
            terminal.send(draft.encode())
            wait(lambda: draft in screen())
            terminal.send(b'\x1dd')
            assert terminal.wait_draining() == 0
            os.kill(int(pid), 0)
            assert tmux('list-sessions', '-F', '#{session_name}') == 'native'
            assert draft in screen(), 'detach lost the native draft'
            send(b'\x15/mo'); time.sleep(.3); send(b'\t')
            wait(lambda: '› /model' in screen())
            print('PASS: detach leaves the original terminal and Codex usable', flush=True)
            send(b'\r')
            wait(lambda: 'Select Model' in screen() or 'Select model' in screen())
            terminal.close()
            terminal = e2e.Terminal(os.environ | {'TERM':'xterm-256color','CODEX24H_MAIL':'0'},
                                    ['attach', '--socket', socket, pid])
            wait(lambda: 'Select Model' in '\n'.join(e2e.screen_text(terminal.output)))
            terminal.send(b'\x1b')
            wait(lambda: 'Select Model' not in screen() and 'Select model' not in screen())
            print('PASS: reattach preserves an already-open native model menu', flush=True)
            long_prompt = '直接输出 100 行测试数据，每行是 STREAM- 前缀加从 1 到 100 的三位补零序号，后面加一段不同的简短中文句子。不要用代码块，不调用工具。'
            terminal.send(long_prompt.encode()); terminal.drain(.3); terminal.send(b'\r')
            wait(lambda: 'esc to interrupt' in screen())
            terminal.send(b'\x1b[5~'); terminal.drain(.3)
            frozen = e2e.screen_text(terminal.output)[:-1]
            wait(lambda: 'STREAM-100' in screen() and 'esc to interrupt' not in screen(), timeout=180)
            terminal.drain(.4)
            assert e2e.screen_text(terminal.output)[:-1] == frozen, 'native streaming pulled the viewport'
            terminal.send(b'\x1db'); terminal.drain(.3)
            print('Native tmux screen state:', tmux('display-message', '-p', '-t', pane,
                  'alternate=#{alternate_on} history=#{history_size} size=#{pane_width}x#{pane_height}'), flush=True)
            views = set()
            for _ in range(25):
                terminal.send(b'\x1b[<64;10;10M'); terminal.drain(.08)
                views.add(tuple(e2e.screen_text(terminal.output)[:-1]))
            if inline:
                assert len(views) >= 5, f'only {len(views)} scrollable native views'
            else:
                assert tmux('display-message', '-p', '-t', pane, '#{alternate_on}') == '1'
                assert 'history limited' in e2e.visible(terminal.output)
            terminal.send(b'\x1b[<65;10;10M'*100)
            wait(lambda: 'attached' in e2e.screen_text(terminal.output)[-1])
            print(f'PASS: 100-line native streaming stayed frozen; {len(views)} distinct wheel views; wheel down restored follow', flush=True)
            if not inline and len(views) < 5:
                print('LIMITATION: full-screen Codex repaints cells without terminal scrollback; attach does not reconstruct chat history', flush=True)
            terminal.send(b'\x1dd')
            assert terminal.wait_draining() == 0
        finally:
            if terminal:
                if sys.exc_info()[0] is not None:
                    artifacts = e2e.ROOT/'test-artifacts'
                    artifacts.mkdir(exist_ok=True)
                    (artifacts/'native-attach-failure.vt').write_bytes(terminal.output)
                terminal.close()
            subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)


if __name__ == '__main__':
    if sys.argv[1:] in (['--native'], ['--native-inline']):
        native_probe(inline=sys.argv[1] == '--native-inline')
    else:
        unittest.main(verbosity=2)
