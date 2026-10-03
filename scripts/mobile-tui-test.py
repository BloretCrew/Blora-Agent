#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise narrow TUI layouts and live resizing in an isolated pseudo-terminal."""
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


with tempfile.TemporaryDirectory(prefix="blora-mobile-tui-") as home:
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
        for width, height in [(40, 24), (36, 12), (24, 10), (100, 30)]:
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.killpg(proc.pid, signal.SIGWINCH)
            drain()
            send(b"/model\r", "供应商")
            send(b"\x1b[C")
            send(b"\x1b[B")
            send(b"\x1b[D")
            send(b"\x1b")
            send(b"/settings\r", "自动批准")
            send(b"\t")
            send(b"\x1b")
            send(b"/help\r", "[session]")
            send(b"\x1b")
            send(b"/")
            for _ in range(10):
                send(b"\x1b[B")
            send(b"\x1b")
            assert proc.poll() is None
        send(b"\x11")
        proc.wait(timeout=5)
        assert proc.returncode == 0
        print("PASS mobile TUI: model picker, settings, command output, slash navigation, live resize at 40x24, 36x12, 24x10, 100x30")
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=5)
        os.close(master)
