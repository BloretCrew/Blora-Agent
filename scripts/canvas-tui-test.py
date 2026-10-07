#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise welcome guidance, message input editing across terminal sizes."""
import fcntl
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


def plain(data):
    text = data.decode(errors="replace")
    text = re.sub(r"\x1b\][^\x07]*\x07", "", text)
    return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text).replace(" ", "")


with tempfile.TemporaryDirectory(prefix="blora-canvas-tui-") as home:
    master, slave = pty.openpty()
    env = dict(os.environ, BLORA_HOME=home, BLORA_PROVIDER="mock", TERM="xterm-256color", NO_COLOR="1")
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 40, 0, 0))
    proc = subprocess.Popen(["target/debug/blora"], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    os.close(slave)

    def drain(duration=0.6):
        result = bytearray()
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    result.extend(os.read(master, 65536))
                except OSError:
                    break
        return plain(result)

    def send(keys, expected=None):
        output = ""
        if keys.startswith(b"/"):
            for key in keys:
                os.write(master, bytes([key]))
                output += drain(0.12)
        else:
            os.write(master, keys)
        output += drain(1.0)
        if expected is not None:
            assert expected in output, (expected, output[-3000:])
        return output

    try:
        assert "欢迎使用Blora" in drain(2)
        send(b"\x1b")
        for width, height in [(24, 10), (36, 12), (40, 24), (100, 30)]:
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.killpg(proc.pid, signal.SIGWINCH)
            drain()
            send(b"\x0e")
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width + 1, 0, 0))
            os.killpg(proc.pid, signal.SIGWINCH)
            drain()
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.killpg(proc.pid, signal.SIGWINCH)
            output = drain()
            assert "输入消息" in output, output[-3000:]
            assert "查看命令" in output or "命令" in output, output[-3000:]
            if width >= 60:
                assert "Ctrl+N" in output, output[-3000:]
            os.write(master, b"canvas-check")
            assert "canvas-check" in drain(), "input did not render"
            send(b"\x7f" * len("canvas-check"))
            send(b"/model\r", "供应商")
            send(b"\x1b")
            assert proc.poll() is None
        send(b"\x11")
        proc.wait(timeout=5)
        assert proc.returncode == 0
        print("PASS canvas TUI: welcome, new session, input editing, model picker and resize at 24x10, 36x12, 40x24, 100x30")
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=5)
        os.close(master)
