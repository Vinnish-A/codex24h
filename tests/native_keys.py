#!/usr/bin/env python3
"""Opt-in real Codex UI checks using existing login (makes model requests).

--attach tests an existing tmux process; --extended adds notes, reconnect,
resize, cancellation, paste/history/completion and a streamed long reply.
--idle-question waits 135 seconds; --scroll-only isolates the resize probe.
"""
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

import e2e


def main():
    attach = '--attach' in sys.argv
    extended = '--extended' in sys.argv
    idle_question = '--idle-question' in sys.argv
    with tempfile.TemporaryDirectory(prefix='codex24h-native-keys-') as directory:
        socket = str(Path(directory)/'tmux.sock')
        def tmux(*args):
            return subprocess.check_output(['tmux', '-S', socket, *args], text=True).strip()
        cmd = ['codex', '--no-alt-screen', '-c', 'notify=[]'] if attach else [str(e2e.BIN)]
        cmd = ['env', 'CODEX24H_MAIL=0', 'CODEX24H_PIN_ROWS=0', *cmd]
        pane, pid = tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'probe', '-x', '100', '-y', '32',
                         '-c', str(e2e.ROOT), '-P', '-F', '#{pane_id} #{pane_pid}', shlex.join(cmd)).split()
        terminal = None
        def screen():
            return tmux('capture-pane', '-p', '-t', pane)
        def send(data):
            if terminal:
                terminal.send(data)
            else:
                tmux('send-keys', '-t', pane, '-H', *[f'{b:02x}' for b in data])
        def wait(predicate, timeout=90):
            deadline = time.monotonic()+timeout
            while time.monotonic()<deadline:
                if terminal:
                    terminal.drain(.08)
                text = screen()
                if predicate(text):
                    return text
                time.sleep(.1)
            raise AssertionError('native key probe timed out:\n'+screen())
        def enter(text):
            send(text.encode() if text.startswith('/') else b'\x1b[200~'+text.encode()+b'\x1b[201~')
            if terminal: terminal.drain(.6)
            else: time.sleep(.6)
            send(b'\r')
            if terminal: terminal.drain(1)
            else: time.sleep(1)
            if text.startswith('/') and any(line.startswith('› '+text) for line in screen().splitlines()):
                send(b'\r')
        def rendered(data):
            # Use tmux's Unicode VT parser for the wrapper's outer screen.
            # The ASCII-only fake-app test decoder leaves ghosts under Chinese.
            raw = Path(directory)/'frame.bin'
            raw.write_bytes(data)
            program = 'import pathlib,sys,time; sys.stdout.buffer.write(pathlib.Path(sys.argv[1]).read_bytes()+b"\\x1b]2;FRAME_READY\\x07"); sys.stdout.buffer.flush(); time.sleep(10)'
            tmux('new-session', '-d', '-s', 'frame', '-x', '100', '-y', '32',
                 shlex.join(['python3', '-c', program, str(raw)]))
            try:
                deadline = time.monotonic()+3
                while tmux('display-message', '-p', '-t', 'frame', '#{pane_title}') != 'FRAME_READY':
                    if time.monotonic() >= deadline:
                        raise AssertionError('outer frame replay did not finish')
                    time.sleep(.03)
                return tmux('capture-pane', '-p', '-t', 'frame').splitlines()[:-1]
            finally:
                tmux('kill-session', '-t', 'frame')
        def long_reply():
            enter('不要调用工具。直接输出80行，每行内容为 LINE-三位序号，例如 LINE-001，直到 LINE-080。不要省略。')
            send(b'\x1b[5~')
            if terminal:
                terminal.drain(.3)
                frozen = rendered(terminal.output)
                terminal.resize(18, 42)
                terminal.drain(.3)
                terminal.resize(32, 100)
                terminal.drain(4)
                after = rendered(terminal.output)
                assert after == frozen, repr([(i, a, b) for i, (a, b) in enumerate(zip(frozen, after)) if a != b])
            else:
                wait(lambda text: 'new updates' in text)
                frozen = screen().splitlines()[:-1]
                tmux('resize-window', '-t', 'probe', '-x', '42', '-y', '18')
                time.sleep(.3)
                tmux('resize-window', '-t', 'probe', '-x', '100', '-y', '32')
                time.sleep(4)
                assert screen().splitlines()[:-1] == frozen
            send(b'\x1db')
            wait(lambda text: any(line.strip() == 'LINE-080' for line in text.splitlines()) and 'Ask Codex' in text)
            print('PASS: real long reply stays frozen; returning live shows completion', flush=True)
        try:
            wait(lambda text: 'Ask Codex' in text)
            if attach:
                terminal = e2e.Terminal(os.environ | {'TERM':'xterm-256color','CODEX24H_MAIL':'0','CODEX24H_PIN_ROWS':'0'},
                                       ['attach','--socket',socket,pid])
                terminal.until('Ask Codex', timeout=30)
                terminal.drain(2)
            # Allow native startup to settle before changing collaboration mode.
            if terminal: terminal.drain(15)
            else: time.sleep(15)
            if '--scroll-only' in sys.argv:
                if terminal: terminal.resize(32, 100); terminal.drain(.5)
                long_reply()
                return
            enter('/plan')
            wait(lambda text: ('Plan mode' in text or 'Plan Mode' in text) and '› /plan' not in text)
            if terminal: terminal.drain(2)
            else: time.sleep(2)
            enter('请调用 request_user_input 一次，同时给我两个单选问题：第一题选择水果，选项为苹果、香蕉、梨；第二题选择颜色，选项为红色、蓝色、绿色。等待我的选择后，只回复各题实际选中选项。不要调用其他工具，不要读写文件。')
            text = wait(lambda text: 'Question 1/2' in text and '香蕉' in text)
            print('QUESTION SCREEN:\n'+text, flush=True)
            if idle_question:
                print('Waiting 135 seconds with a real question unanswered', flush=True)
                deadline = time.monotonic()+135
                while time.monotonic() < deadline:
                    if terminal: terminal.drain(.2)
                    else: time.sleep(.2)
                assert 'Question 1/2' in screen(), 'native question disappeared without an answer:\n'+screen()
                print('PASS: native question remains unanswered after 135 seconds', flush=True)
            send(b'\x1b[C')
            wait(lambda text: 'Question 2/2' in text)
            send(b'\x1b[D')
            wait(lambda text: 'Question 1/2' in text)
            send(b'\x1b[B')
            wait(lambda text: any('› 2.' in line and '香蕉' in line for line in text.splitlines()))
            if extended:
                send(b'\t')
                wait(lambda text: 'notes' in text.lower())
                send(b'\x1b[200~'+ '中文备注：周五交付🙂'.encode()+b'\x1b[201~')
                wait(lambda text: '周五交付' in text)
                print('NOTES SCREEN:\n'+screen(), flush=True)
                send(b'\x1b[Z')
            send(b'\r')
            wait(lambda text: 'Question 2/2' in text)
            if extended:
                # Search is local even while a real model question waits.
                send(b'\x1d/never-submit-this\x1b')
                if terminal: terminal.drain(.4)
                else: time.sleep(.4)
                send(b'\x1db')
                wait(lambda text: 'Question 2/2' in text)
                if attach:
                    send(b'\x1dd')
                    assert terminal.wait_draining() == 0
                    terminal.close()
                    os.kill(int(pid), 0)
                    terminal = e2e.Terminal(os.environ | {'TERM':'xterm-256color','CODEX24H_MAIL':'0','CODEX24H_PIN_ROWS':'0'},
                                           ['attach','--socket',socket,pid])
                    terminal.until('Question 2/2', timeout=30)
                    terminal.resize(18, 42)
                else:
                    tmux('resize-window', '-t', 'probe', '-x', '42', '-y', '18')
                wait(lambda text: 'Question 2/2' in text and '蓝色' in text)
                print('PASS: local search, pending question reconnect/resize', flush=True)
            send(b'\x1b[B')
            wait(lambda text: any('› 2.' in line and '蓝色' in line for line in text.splitlines()))
            if extended:
                if terminal: terminal.resize(32, 100)
                else: tmux('resize-window', '-t', 'probe', '-x', '100', '-y', '32')
            send(b'\r')
            text = wait(lambda text: 'Question 2/2' not in text and '香蕉' in text.rsplit('•', 1)[-1] and '蓝色' in text.rsplit('•', 1)[-1] and 'Ask Codex' in text)
            if extended:
                assert '周五' in text, 'selected option note missing from native answer transcript'
                # A paste immediately followed by Enter must remain usable.
                send(b'\x1b[200~'+ '不要调用工具，只回复数字：17 加 26 等于多少？'.encode()+b'\x1b[201~\r')
                wait(lambda text: '43' in text.rsplit('•', 1)[-1] and 'Ask Codex' in text)
                send(b'\x1b[A')
                wait(lambda text: any(line.startswith('› ') and '17' in line for line in text.splitlines()))
                send(b'\x1b[B')
                wait(lambda text: 'Ask Codex' in text)
                send(b'/mo')
                wait(lambda text: '/model' in text and 'choose' in text)
                send(b'\t')
                wait(lambda text: any(line.startswith('› /model') for line in text.splitlines()))
                send(b'\x15')
                wait(lambda text: 'Ask Codex' in text)
                question = '请用 request_user_input 问一个单选问题：请选择下一步，选项为仅预览、执行、稍后。收到选择后只回复选项名，不要使用其他工具或执行任何操作。'
                enter(question)
                wait(lambda text: '› 1.' in text and '仅预览' in text and 'enter to submit' in text.lower())
                send(b'\x1b')
                wait(lambda text: 'Ask Codex' in text and 'enter to submit' not in text.lower())
                enter(question)
                wait(lambda text: '› 1.' in text and '仅预览' in text and 'enter to submit' in text.lower())
                send(b'\r')
                wait(lambda text: '仅预览' in text.rsplit('•', 1)[-1] and 'Ask Codex' in text)
                print('PASS: Escape cancels native question; next question submits normally', flush=True)
                long_reply()
                print('PASS: notes, immediate paste+Enter, next-turn history and completion', flush=True)
            artifacts = e2e.ROOT/'test-artifacts'
            artifacts.mkdir(exist_ok=True)
            (artifacts/('native-keys-attach.txt' if attach else 'native-keys-direct.txt')).write_text(text)
            print('PASS: native question navigation, selection and submission', flush=True)
        finally:
            if terminal: terminal.close()
            subprocess.run(['tmux','-S',socket,'kill-server'],capture_output=True)


if __name__ == '__main__':
    main()
