#!/usr/bin/env python3
"""Local end-to-end checks: python3 tests/e2e.py (after cargo build)."""

import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("CODEX24H_TEST_BIN", ROOT / "target/debug/codex24h"))
FAKE = Path(__file__).with_name("fake_codex.py")


def visible(data):
    """Extract printable UTF-8 in output order; enough to find screen markers."""
    data = re.sub(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", b"", data)
    data = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", data)
    data = re.sub(rb"\x1b[()][0-9A-Za-z]", b"", data)
    return bytes(c for c in data if c >= 32 or c in (10, 13)).decode("utf-8", "replace")


def screen_text(data, rows=24, cols=80):
    """Rebuild the ASCII viewport from the wrapper's cursor-addressed output."""
    cells = [[" "] * cols for _ in range(rows)]
    row = col = 0
    i = 0
    while i < len(data):
        byte = data[i]
        if byte == 27 and i + 1 < len(data):
            kind = data[i + 1]
            if kind == ord("["):
                end = i + 2
                while end < len(data) and not 64 <= data[end] <= 126:
                    end += 1
                if end == len(data):
                    break
                params = data[i + 2:end].decode("ascii", "ignore")
                final = chr(data[end])
                if final in "Hf":
                    parts = params.split(";")
                    row = max(0, min(rows - 1, int(parts[0] or "1") - 1))
                    col = max(0, min(cols - 1, int(parts[1] or "1") - 1)) if len(parts) > 1 else 0
                elif final == "J" and params in ("", "2"):
                    cells = [[" "] * cols for _ in range(rows)]
                elif final == "K":
                    cells[row][col:] = [" "] * (cols - col)
                elif final == "h" and params == "?1049":
                    cells = [[" "] * cols for _ in range(rows)]
                i = end + 1
                continue
            if kind in (ord("]"), ord("P"), ord("_"), ord("^")):
                start = i + 2
                bel = data.find(b"\x07", start) if kind == ord("]") else -1
                st = data.find(b"\x1b\\", start)
                ends = [(bel, 1), (st, 2)]
                ends = [(pos, width) for pos, width in ends if pos >= 0]
                if not ends:
                    break
                pos, width = min(ends)
                i = pos + width
                continue
            i += 2
            continue
        if byte == 13:
            col = 0
        elif byte == 10:
            row = min(rows - 1, row + 1)
        elif 32 <= byte < 127:
            cells[row][col] = chr(byte)
            col = min(cols - 1, col + 1)
        i += 1
    return ["".join(line).rstrip() for line in cells]


class Terminal:
    def __init__(self, env, args=()):
        self.master, slave = pty.openpty()
        self.slave = slave
        self.set_size(slave, 24, 80)
        self.initial_attrs = termios.tcgetattr(slave)
        self.initial_flags = fcntl.fcntl(slave, fcntl.F_GETFL)

        def controlling_tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen(
            [str(BIN), *args], stdin=slave, stdout=slave, stderr=slave,
            env=env, cwd=ROOT, preexec_fn=controlling_tty,
        )
        self.output = bytearray()

    @staticmethod
    def set_size(fd, rows, cols):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def resize(self, rows, cols):
        self.set_size(self.master, rows, cols)
        os.kill(self.process.pid, signal.SIGWINCH)

    def send(self, data):
        os.write(self.master, data)

    def drain(self, seconds=0.2):
        end = time.monotonic() + seconds
        received = bytearray()
        while time.monotonic() < end:
            ready, _, _ = select.select([self.master], [], [], min(0.03, end - time.monotonic()))
            if not ready:
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError as exc:
                if exc.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            received.extend(chunk)
        self.output.extend(received)
        return bytes(received)

    def until(self, needle, timeout=5, since=0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.drain(0.05)
            if needle in visible(self.output[since:]):
                return
            if self.process.poll() is not None:
                break
        raise AssertionError(
            f"Timed out waiting for {needle!r}; exit={self.process.poll()}; "
            f"visible tail={visible(self.output[since:])[-300:]!r}"
        )

    def wait_draining(self, timeout=5):
        """Keep acting as a terminal emulator while the wrapper exits."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.drain(0.05)
            status = self.process.poll()
            if status is not None:
                return status
        raise AssertionError(
            f"wrapper did not exit; state={process_state(self.process.pid)}, "
            f"output tail={visible(self.output)[-200:]!r}"
        )

    def close(self):
        if self.process.poll() is None:
            os.kill(self.process.pid, signal.SIGCONT)
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        os.close(self.master)
        os.close(self.slave)

    def assert_restored(self, testcase):
        testcase.assertEqual(termios.tcgetattr(self.slave), self.initial_attrs)
        testcase.assertEqual(fcntl.fcntl(self.slave, fcntl.F_GETFL), self.initial_flags)


def process_state(pid):
    try:
        return Path(f"/proc/{pid}/status").read_text().split("State:\t", 1)[1].split()[0]
    except FileNotFoundError:
        return None


def process_metrics(pid):
    """Linux-only diagnostic numbers, never used as timing thresholds."""
    fields = Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()
    cpu = (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    status = Path(f"/proc/{pid}/status").read_text()
    rss = int(re.search(r"^VmRSS:\s+(\d+) kB$", status, re.MULTILINE).group(1))
    return cpu, rss


class WrapperE2E(unittest.TestCase):
    def setUp(self):
        self.assertTrue(BIN.is_file(), f"Build first: cargo build ({BIN} absent)")
        self.temp = tempfile.TemporaryDirectory(prefix="codex24h-e2e-")
        self.addCleanup(self.temp.cleanup)
        self.env = os.environ.copy()
        self.env.update(
            CODEX24H_CODEX=str(FAKE), FAKE_LOG_DIR=self.temp.name,
            CODEX24H_HISTORY="40", CODEX24H_MAIL="0", CODEX24H_PIN_ROWS="0", TERM="xterm-256color",
        )

    def log(self, name):
        path = Path(self.temp.name, name)
        return path.read_bytes() if path.exists() else b""

    def wait_log(self, name, needle, timeout=5):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if needle in self.log(name):
                return
            time.sleep(0.02)
        self.fail(f"{needle!r} absent from {name}: {self.log(name)!r}")

    def wait_event_count(self, event, count, timeout=5):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if self.log("events.log").count(event) >= count:
                return
            time.sleep(0.02)
        self.fail(f"{event!r} appeared fewer than {count} times")

    def wait_state(self, pid, state, timeout=5):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if process_state(pid) == state:
                return
            time.sleep(0.02)
        self.fail(f"PID {pid} never entered state {state}; now {process_state(pid)}")

    def child_pid(self):
        self.wait_log("pid.log", b"\n")
        return int(self.log("pid.log").splitlines()[-1])

    def assert_child_gone(self, pid):
        end = time.monotonic() + 3
        while time.monotonic() < end and process_state(pid) is not None:
            time.sleep(0.02)
        self.assertIsNone(process_state(pid), f"child PID {pid} survived wrapper exit")

    def test_streaming_survives_drag_release_and_lost_mouseup(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until('READY-080')
        terminal.send(b':heartbeat 100\n')
        self.wait_log('events.log', b'beat:1\n')
        def beat():
            terminal.drain(.2)
            lines = screen_text(terminal.output)
            match = re.search(r'WORK-(\d+)', '\n'.join(lines))
            return int(match[1]) if match else 0
        for _ in range(4):
            terminal.send(b'\x1b[<0;1;1M\x1b[<32;5;1M\x1b[<0;5;1m')
            before = beat()
            self.assertGreater(beat(), before, 'timer must continue after mouse-up without Ctrl+C')
        terminal.send(b'\x1b[<0;1;1M\x1b[<32;5;1M')
        terminal.send(b'\x1b[A\t')  # release outside the client: next key still reaches Codex
        self.wait_log('stdin.bin', b'\x1b[A\t')
        before = beat()
        self.assertGreater(beat(), before)
        terminal.send(b'\x03')
        self.wait_log('stdin.bin', b'\x03')
        terminal.send(b':exit 0\n')
        self.assertEqual(terminal.wait_draining(), 0)

    def test_transient_tiny_resize_preserves_running_child(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until('READY')
        child = self.child_pid()
        for rows, cols in ((0, 0), (1, 80), (24, 1)):
            terminal.resize(rows, cols)
            terminal.drain(.15)
            self.assertIsNone(terminal.process.poll(), 'temporary terminal size terminated wrapper')
            os.kill(child, 0)
        terminal.resize(24, 80)
        terminal.send(b'after-resize')
        terminal.drain(.3)
        self.assertIn(b'after-resize', self.log('stdin.bin'))

    def test_direct_pipe_preserves_bytes_args_and_status(self):
        env = dict(self.env, FAKE_MODE="direct", FAKE_EXIT="17")
        original = b"a\0b\xff\n"
        command = [str(BIN), "exec", "--json", "雪"]
        result = subprocess.run(command, input=original, capture_output=True, env=env, cwd=ROOT)
        self.assertEqual(result.returncode, 17)
        self.assertEqual(result.stdout, b"STDOUT\0" + original)
        self.assertEqual(result.stderr, b"STDERR\0" + original)
        self.assertEqual(json.loads(self.log("argv.json")), command[1:])

        help_result = subprocess.run(
            [str(BIN), "--help"], input=b"help\n", capture_output=True, env=env, cwd=ROOT,
        )
        self.assertEqual(help_result.returncode, 17)
        self.assertEqual(help_result.stdout, b"STDOUT\0help\n")
        self.assertEqual(json.loads(self.log("argv.json")), ["--help"])

        # A prompt or resume on a pipe stays a direct Codex invocation too.
        for arguments in (["请检查代码"], ["resume", "--last"]):
            piped = subprocess.run(
                [str(BIN), *arguments], input=b"pipe", capture_output=True, env=env, cwd=ROOT,
            )
            self.assertEqual((piped.returncode, piped.stdout, piped.stderr),
                             (17, b"STDOUT\0pipe", b"STDERR\0pipe"))
            self.assertEqual(json.loads(self.log("argv.json")), arguments)

    def test_pty_browse_continues_child_and_recovers_live_view(self):
        terminal = Terminal(self.env, ["resume", "--last"])
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        self.assertEqual(
            json.loads(self.log("argv.json")), ["--no-alt-screen", "resume", "--last"]
        )

        # Child queries are answered from the virtual terminal. The outer
        # terminal has a different coordinate system and must not supply CPR.
        self.wait_log("stdin.bin", b"\x1b[23;1R")
        self.wait_log("stdin.bin", b"\x1b[?6c")
        replies = self.log("stdin.bin")
        self.assertRegex(replies, rb"\x1b\]10;rgb:[0-9a-fA-F]+/[0-9a-fA-F]+/[0-9a-fA-F]+")
        outer_replies = b"\x1b[?1;2c\x1b]10;rgb:ffff/ffff/ffff\x07\x1b[?0u"
        terminal.send(outer_replies)
        terminal.drain(0.2)
        self.assertEqual(self.log("stdin.bin"), replies, "outer query replies leaked to child")
        # CSI 1;2 R is also the native Shift-F3 key. The wrapper must pass it.
        terminal.send(b"\x1b[1;2R")
        self.wait_log("stdin.bin", b"\x1b[1;2R")

        # Bracketed paste and Ctrl-C are application input, not wrapper commands.
        paste = b"\x1b[200~a/b?\x1b[201~"
        terminal.send(paste + b"\x03")
        self.wait_log("stdin.bin", paste + b"\x03")

        terminal.send(b"\x1b[5~")  # Page Up: browse older output.
        terminal.drain(0.3)
        stdin_before_wheel = self.log("stdin.bin")
        terminal.send(b"\x1b[<64;20;10M")  # Mouse wheel also browses locally.
        terminal.drain(0.2)
        self.assertEqual(self.log("stdin.bin"), stdin_before_wheel)
        terminal.send(b":burst 120\n")  # Exceeds the 40-row live history limit.
        self.wait_log("events.log", b"burst:120")
        frozen = terminal.drain(0.4)
        self.assertNotIn("NEW-120", visible(frozen), "live chat content repainted during browse")

        preserved = screen_text(terminal.output)[:-1]
        terminal.resize(10, 30)
        terminal.drain(.2)
        terminal.resize(24, 80)
        terminal.drain(.2)
        self.assertEqual(screen_text(terminal.output)[:-1], preserved,
                         "shrinking then restoring erased frozen content")

        terminal.send(b":repaint\n:clear\n")
        self.wait_log("events.log", b"clear")
        frozen = terminal.drain(0.3)
        self.assertNotIn("REPAINTED", visible(frozen))
        self.assertNotIn("AFTER-CLEAR", visible(frozen))

        latest = len(terminal.output)
        terminal.send(b"\x1d" + b"b")  # Ctrl-] b: bottom / follow.
        terminal.until("AFTER-CLEAR", since=latest)

        terminal.resize(31, 92)
        self.wait_log("size.log", b"30x92")  # Last outer row belongs to wrapper status.
        latest = len(terminal.output)
        terminal.send(b":repaint\n")
        terminal.until("REPAINTED", since=latest)
        terminal.send(b":exit 23\n")
        self.assertEqual(terminal.process.wait(timeout=5), 23)

    def test_same_read_search_is_local_and_exit_keeps_browse_open(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        before = self.log("stdin.bin")
        terminal.send(b"\x1d/ROW-070")  # Prefix and query arrive in one read.
        terminal.until("/ROW-070")
        self.assertEqual(self.log("stdin.bin"), before, "search text leaked to Codex")
        terminal.send(b"\x1b")  # Search -> Browse.
        terminal.drain(0.3)
        terminal.send(b"\x1b[5~")
        terminal.drain(0.2)
        terminal.send(b":exit 23\n")
        self.wait_log("events.log", b"exit")
        terminal.until("Codex exited (23)")
        time.sleep(0.6)
        self.assertIsNone(terminal.process.poll(), "browse view closed with child")
        terminal.send(b"\x1b[5~")
        terminal.drain(0.2)
        terminal.resize(25, 81)  # Full repaint makes the preserved rows observable.
        self.assertIn("ROW-", visible(terminal.drain(0.4)))
        terminal.send(b"q")
        self.assertEqual(terminal.process.wait(timeout=5), 23)

    def test_external_signals_restore_terminal_and_reap_child(self):
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=sig):
                terminal = Terminal(self.env)
                try:
                    terminal.until("READY-080")
                    child = self.child_pid()
                    os.kill(terminal.process.pid, sig)
                    self.assertEqual(terminal.process.wait(timeout=5), 128 + sig)
                    terminal.assert_restored(self)
                    self.assert_child_gone(child)
                finally:
                    terminal.close()

    def test_child_self_signal_propagates_status_and_restores_tty(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        child = self.child_pid()
        terminal.send(b":signal 15\n")
        self.wait_log("events.log", b"signal:15")
        self.assertEqual(terminal.process.wait(timeout=5), 143)
        terminal.assert_restored(self)
        self.assert_child_gone(child)

    def test_suspend_restores_shell_tty_and_resume_child(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        child = self.child_pid()
        os.kill(terminal.process.pid, signal.SIGTSTP)
        self.wait_state(terminal.process.pid, "T")
        self.wait_state(child, "T")
        terminal.assert_restored(self)
        os.kill(terminal.process.pid, signal.SIGCONT)
        end = time.monotonic() + 5
        while time.monotonic() < end:
            if process_state(child) != "T" and not (termios.tcgetattr(terminal.slave)[3] & termios.ICANON):
                break
            time.sleep(0.02)
        else:
            self.fail("wrapper or child did not resume in raw mode")
        terminal.send(b":burst 1\n")
        self.wait_log("events.log", b"burst:1")
        terminal.send(b":exit 0\n")
        self.wait_log("events.log", b"exit")
        self.assertEqual(terminal.wait_draining(), 0)
        terminal.assert_restored(self)
        self.assert_child_gone(child)

    def test_flood_drains_child_while_outer_terminal_is_unread(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        initial_cpu, _ = process_metrics(terminal.process.pid)
        start = time.monotonic()
        terminal.send(b":flood 2\n")
        # Deliberately do not read the outer PTY while the child writes 2 MiB.
        self.wait_log("events.log", b"flood:2:done", timeout=12)
        elapsed = time.monotonic() - start
        final_cpu, rss = process_metrics(terminal.process.pid)
        self.assertIsNone(terminal.process.poll())
        terminal.send(b":burst 1\n")
        self.wait_log("events.log", b"burst:1", timeout=5)
        terminal.drain(0.3)
        terminal.send(b":exit 0\n")
        self.assertEqual(terminal.process.wait(timeout=5), 0)
        print(
            f"\n  2 MiB child flood: {elapsed:.2f}s wall, "
            f"{final_cpu - initial_cpu:.2f}s wrapper CPU, {rss} KiB wrapper RSS "
            "with outer PTY unread"
        )

    def test_native_style_question_keys_reach_child_in_live_mode(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")

        def ask(keys, expected):
            count = self.log("events.log").count(b"question:open") + 1
            terminal.send(b":question\n")
            self.wait_event_count(b"question:open", count)
            terminal.drain(0.2)
            self.assertIn("QUESTION:Choose a review path", screen_text(terminal.output)[0])
            before = len(self.log("stdin.bin"))
            terminal.send(keys)
            self.wait_log("answer.log", expected)
            self.assertIn(keys, self.log("stdin.bin")[before:])

        ask(b"\x1b[B\r", b"option:Review changes")
        ask(b"\t\t\r", b"option:Cancel")
        ask(b"1\r", b"option:Approve edits")
        ask(b"\x1b", b"cancel")
        ask(b"4\x1b[200~Please explain first\x1b[201~\r", b"free:Please explain first")
        terminal.send(b":exit 0\n")
        self.assertEqual(terminal.wait_draining(), 0)

    def test_modified_shortcuts_remain_exact_without_tmux(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        terminal.drain(.2)
        before = len(self.log("stdin.bin"))
        keys = b"\x1b[1;2D\x1b[1;2C\x1b[Z\x1b[13;2u\x1b[27;2;13~\x1b[5;2~\x1b[6;5~"
        for start in range(0, len(keys), 3):
            terminal.send(keys[start:start+3])
            terminal.drain(.02)
        self.wait_log("stdin.bin", keys)
        self.assertEqual(self.log("stdin.bin")[before:], keys)

    def test_question_frozen_then_revealed_and_answered(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        terminal.send(b"\x1b[5~")
        terminal.drain(0.2)
        frozen_body = screen_text(terminal.output)[:-1]
        terminal.send(b":question\n")
        self.wait_log("events.log", b"question:open")
        terminal.drain(0.3)
        self.assertEqual(screen_text(terminal.output)[:-1], frozen_body)
        terminal.send(b"\x1db")
        terminal.drain(0.3)
        self.assertIn("QUESTION:Choose a review path", screen_text(terminal.output)[0])
        terminal.send(b"2\r")
        self.wait_log("answer.log", b"option:Review changes")
        terminal.send(b":exit 0\n")
        self.assertEqual(terminal.wait_draining(), 0)

    def test_search_and_copy_keys_cannot_answer_child_question(self):
        terminal = Terminal(self.env)
        self.addCleanup(terminal.close)
        terminal.until("READY-080")
        terminal.send(b":question\n")
        self.wait_log("events.log", b"question:open")
        terminal.drain(0.2)
        self.assertIn("QUESTION:Choose a review path", screen_text(terminal.output)[0])
        before = self.log("stdin.bin")
        terminal.send(b"\x1d/ROW-070")
        terminal.until("/ROW-070")
        terminal.send(b"\rjk")  # Search accepts; j/k stay in local Copy mode.
        terminal.drain(0.2)
        self.assertEqual(self.log("stdin.bin"), before)
        self.assertEqual(self.log("answer.log"), b"")
        terminal.send(b"\x1b")
        terminal.drain(0.2)
        terminal.send(b"\x1db")
        terminal.drain(0.2)
        terminal.send(b"1\r")
        self.wait_log("answer.log", b"option:Approve edits")

        count = self.log("events.log").count(b"question:open") + 1
        terminal.send(b":question\n")
        self.wait_event_count(b"question:open", count)
        terminal.drain(0.2)
        before = self.log("stdin.bin")
        terminal.send(b"\x1d[jk")  # Enter Copy directly; navigation is local.
        terminal.drain(0.2)
        self.assertEqual(self.log("stdin.bin"), before)
        self.assertEqual(self.log("answer.log").count(b"option:"), 1)
        terminal.send(b"\x1b")
        terminal.drain(0.2)
        terminal.send(b"\x1db")
        terminal.drain(0.2)
        terminal.send(b"3\r")
        self.wait_log("answer.log", b"option:Cancel")
        terminal.send(b":exit 0\n")
        self.assertEqual(terminal.wait_draining(), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
