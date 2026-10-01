#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise first-run TUI with a real pseudo-terminal and isolated Blora home."""
import fcntl
import os
import pty
import re
import select
import signal
import sqlite3
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path


def text(output):
    value = output.decode(errors='replace')
    value = re.sub(r'\x1b\][^\x07]*\x07', '', value)
    return re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', value)


def exercise(home, keys, expected, size=(100, 30)):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', size[1], size[0], 0, 0))
    env = dict(os.environ, BLORA_HOME=str(home), TERM='xterm-256color', NO_COLOR='1', BLORA_PROVIDER='mock')
    proc = subprocess.Popen(['target/debug/blora'], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    os.close(slave)
    output = bytearray()

    def drain(duration):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    break

    try:
        drain(2)
        assert expected.replace(' ', '') in text(output).replace(' ', ''), (expected, output[-2000:].decode(errors='replace'))
        for key in keys:
            os.write(master, key)
            drain(0.4)
        os.write(master, b'\x11')  # Ctrl+Q also works inside onboarding.
        drain(0.5)
        if proc.poll() is None:
            os.write(master, b'\x03')
            drain(0.5)
        proc.wait(timeout=5)
        assert proc.returncode == 0, output[-2000:].decode(errors='replace')
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait()
        os.close(master)
    return output


with tempfile.TemporaryDirectory(prefix='blora-oobe-tui-') as directory:
    home = Path(directory)
    exercise(home, [], '欢迎使用Blora')
    exercise(home, [b'\r', b'\t', b'\r', b'\r'], '欢迎使用Blora')
    databases = list(home.glob('*.sqlite*'))
    database = next(path for path in databases if not path.name.endswith(('-wal', '-shm')))
    with sqlite3.connect(database) as conn:
        assert conn.execute("SELECT version FROM onboarding WHERE surface='tui'").fetchone() == (1,)
    output = exercise(home, [b'/onboarding\r'], 'PassPort')
    assert '欢迎使用Blora' in text(output).replace(' ', ''), text(output)[-2500:]
    output = exercise(home, [b'/login\r'], 'PassPort')
    assert '连接账号' in text(output)
    output = exercise(home, [b'/onboarding\r'], '终端较小', size=(20, 6))
    assert '欢迎使用Blora' in text(output).replace(' ', ''), text(output)[-2500:]
    output = exercise(home, [b'/settings\r', b'\x1b[B', b'\x1b[B', b'\x1b[B', b'\x1b[B', b'\r'], 'PassPort')
    assert '欢迎使用Blora' in text(output).replace(' ', ''), text(output)[-2500:]
    print('PASS TUI: first run, interrupted onboarding, skip, persisted completion, restart, /onboarding, /login, small terminal')
