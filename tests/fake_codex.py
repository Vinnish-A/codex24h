#!/usr/bin/env python3
"""Small, controllable Codex stand-in for local PTY tests."""

import fcntl
import json
import os
import re
import select
import signal
import struct
import sys
import termios
import tty


def write_log(name, data):
    directory = os.environ["FAKE_LOG_DIR"]
    with open(os.path.join(directory, name), "ab") as log:
        log.write(data)


def emit(data):
    while data:
        data = data[os.write(sys.stdout.fileno(), data):]


def winsize():
    packed = fcntl.ioctl(sys.stdin.fileno(), termios.TIOCGWINSZ, b"\0" * 8)
    rows, cols, _, _ = struct.unpack("HHHH", packed)
    write_log("size.log", f"{rows}x{cols}\n".encode())
    emit(f"SIZE:{rows}x{cols}\r\n".encode())


CHOICES = ("Approve edits", "Review changes", "Cancel", "Other")


def draw_question(focus, free_text):
    lines = ["QUESTION:Choose a review path"]
    for i, choice in enumerate(CHOICES):
        lines.append(f"{'>' if focus == i else ' '} ({'o' if focus == i else ' '}) {i + 1}. {choice}")
    lines.append(f"Free text: {free_text}")
    lines.append("Arrow keys / Tab / 1-4 / Enter / Escape")
    emit(b"\x1b[2J\x1b[H" + "\r\n".join(lines).encode() + b"\r\n")


def question_input(pending, state, timeout=False):
    """Consume complete native-style menu keys; keep split escape sequences."""
    while pending:
        if pending.startswith(b"\x1b[200~"):
            end = pending.find(b"\x1b[201~", 6)
            if end < 0:
                return True
            state["focus"] = 3
            state["text"] += pending[6:end].decode("utf-8", "replace")
            del pending[:end + 6]
            draw_question(state["focus"], state["text"])
            continue
        if pending[0] == 27:
            if len(pending) == 1:
                if not timeout:
                    return True
                pending.clear()
                write_log("answer.log", b"cancel\n")
                emit(b"ANSWER:CANCELLED\r\n")
                return False
            if pending[1] == ord("["):
                if len(pending) < 3:
                    return True
                if pending[2] in (ord("A"), ord("B")):
                    step = -1 if pending[2] == ord("A") else 1
                    state["focus"] = (state["focus"] + step) % len(CHOICES)
                    del pending[:3]
                    draw_question(state["focus"], state["text"])
                    continue
                # Other complete CSI keys are ignored by this fixture.
                match = re.match(rb"\x1b\[[0-?]*[ -/]*[@-~]", pending)
                if match is None:
                    return True
                del pending[:match.end()]
                continue
            del pending[0]
            write_log("answer.log", b"cancel\n")
            emit(b"ANSWER:CANCELLED\r\n")
            return False
        key = pending.pop(0)
        if key in (10, 13):
            answer = (f"free:{state['text']}" if state["focus"] == 3
                      else f"option:{CHOICES[state['focus']]}")
            write_log("answer.log", (answer + "\n").encode())
            emit(f"ANSWER:{answer}\r\n".encode())
            return False
        if key == 9:
            state["focus"] = (state["focus"] + 1) % len(CHOICES)
            draw_question(state["focus"], state["text"])
        elif ord("1") <= key <= ord("4"):
            state["focus"] = key - ord("1")
            draw_question(state["focus"], state["text"])
        elif state["focus"] == 3 and 32 <= key < 127:
            state["text"] += chr(key)
            draw_question(state["focus"], state["text"])
    return True


def interactive():
    tty.setraw(sys.stdin.fileno())
    write_log("pid.log", f"{os.getpid()}\n".encode())
    signal.signal(signal.SIGWINCH, lambda _signal, _frame: winsize())
    winsize()
    emit(b"\x1b[?2004h")
    emit(b"".join(f"ROW-{i:03d}\r\n".encode() for i in range(1, 81)))
    emit(b"READY-080\r\n")
    # Queries normally emitted by a terminal application. The wrapper answers
    # these from its virtual screen, independent of the outer PTY.
    emit(b"\x1b[6n\x1b[0c\x1b]10;?\x07\x1b[?u")
    all_input = bytearray()
    processed = 0
    question = None
    question_pending = bytearray()
    question_cursor = 0
    while True:
        ready, _, _ = select.select([sys.stdin.fileno()], [], [], 0.1)
        if not ready:
            if question is not None and question_pending == b"\x1b":
                question_input(question_pending, question, timeout=True)
                question = None
            continue
        data = os.read(sys.stdin.fileno(), 4096)
        if not data:
            return 0
        write_log("stdin.bin", data)
        all_input.extend(data)
        for match in re.finditer(rb":(burst|repaint|clear|exit|signal|flood|question|alternate)[ \t]*(\d*)[\r\n]", all_input):
            if match.end() <= processed:
                continue
            processed = match.end()
            command, count = match.group(1), match.group(2)
            if command == b"burst":
                n = int(count or b"1")
                emit(b"".join(f"NEW-{i:03d}\r\n".encode() for i in range(1, n + 1)))
                write_log("events.log", f"burst:{n}\n".encode())
            elif command == b"repaint":
                emit(b"\x1b[2J\x1b[HREPAINTED\x1b[2;1Hsecond row\r\n")
                write_log("events.log", b"repaint\n")
            elif command == b"clear":
                emit(b"\x1b[3J\x1b[2J\x1b[HAFTER-CLEAR\r\n")
                write_log("events.log", b"clear\n")
            elif command == b"exit":
                write_log("events.log", b"exit\n")
                return int(count or b"0")
            elif command == b"signal":
                n = int(count)
                write_log("events.log", f"signal:{n}\n".encode())
                os.kill(os.getpid(), n)
            elif command == b"flood":
                mib = int(count or b"2")
                block = b"".join(
                    f"\x1b[1;1HFLOOD-{i:04d} ".encode() + b"." * 64
                    for i in range(256)
                )
                remaining = mib * 1024 * 1024
                while remaining:
                    chunk = block[:remaining]
                    emit(chunk)
                    remaining -= len(chunk)
                write_log("events.log", f"flood:{mib}:done\n".encode())
            elif command == b"question":
                question = {"focus": 0, "text": ""}
                question_pending.clear()
                question_cursor = match.end()
                write_log("events.log", b"question:open\n")
                draw_question(0, "")
            elif command == b"alternate":
                emit(b"\x1b[?1049h\x1b[HALT-READY\r\n")
        if question is not None:
            question_pending.extend(all_input[question_cursor:])
            question_cursor = len(all_input)
            if not question_input(question_pending, question):
                question = None


def main():
    with open(os.path.join(os.environ["FAKE_LOG_DIR"], "argv.json"), "w", encoding="utf-8") as log:
        json.dump(sys.argv[1:], log, ensure_ascii=False)
    if os.environ.get("FAKE_MODE") == "direct":
        stdin = sys.stdin.buffer.read()
        sys.stdout.buffer.write(b"STDOUT\0" + stdin)
        sys.stderr.buffer.write(b"STDERR\0" + stdin)
        return int(os.environ.get("FAKE_EXIT", "0"))
    return interactive()


if __name__ == "__main__":
    sys.exit(main())
