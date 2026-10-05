#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise harness selection and fake external requests without account access."""
import fcntl
import json
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


def plain(data):
    value = data.decode(errors="replace")
    value = re.sub(r"\x1b\][^\x07]*\x07", "", value)
    return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", value).replace(" ", "")


with tempfile.TemporaryDirectory(prefix="blora-harness-tui-") as directory:
    root = Path(directory)
    binary = Path("target/debug/blora").resolve()
    cli = root / "bin"
    cli.mkdir()
    (cli / "kill").symlink_to("/usr/bin/kill")
    fake = cli / "claude"
    fake.write_text("""#!/usr/bin/python3
import json, os, sys, time
with open(os.environ['FAKE_CLI_LOG'], 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\\n')
print(json.dumps({'type':'system','session_id':'01a10c01-7571-71c2-9f0f-62886fc7be5b','model':'fake-model'}), flush=True)
time.sleep(2)
print(json.dumps({'type':'assistant','message':{'id':str(time.time()),'content':[{'type':'text','text':'FAKE_REPLY'}]}}), flush=True)
print(json.dumps({'type':'result','subtype':'success','session_id':'01a10c01-7571-71c2-9f0f-62886fc7be5b','result':'FAKE_REPLY'}), flush=True)
""")
    fake.chmod(0o755)
    log = root / "cli.jsonl"
    for locale in ("zh-CN", "en"):
        home = root / locale
        log.write_text("")
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock", PATH=str(cli),
                   TERM="xterm-256color", NO_COLOR="1", BLORA_LOCALE=locale,
                   BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1", FAKE_CLI_LOG=str(log))
        proc = subprocess.Popen([str(binary)], cwd=root, stdin=slave, stdout=slave,
                                stderr=slave, env=env, start_new_session=True)
        os.close(slave)

        def drain(duration=0.6):
            data = bytearray()
            deadline = time.monotonic() + duration
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        data.extend(os.read(master, 65536))
                    except OSError:
                        break
            return plain(data)

        def send(keys, duration=0.7):
            os.write(master, keys)
            return drain(duration)

        def events(kind):
            database = next(path for path in home.glob("*.sqlite*") if not path.name.endswith(("-wal", "-shm")))
            with sqlite3.connect(database) as conn:
                return [json.loads(row[0]) for row in conn.execute(
                    "SELECT payload FROM events WHERE event_type=? ORDER BY sequence", (kind,))]

        try:
            drain(2)
            send(b"\x1b")
            # Keyboard entry, missing item rejection, input isolation and window restore.
            output = send(b"\x1b[<0;3;1M\x1b[<0;3;1m")
            assert "ClaudeCode" in output, output
            assert ("未安装" if locale == "zh-CN" else "Notinstalled") in output, output
            send(b"\x1b[<0;15;8M\x1b[<0;15;8m")
            send(b"IGNORED")
            send(b"\r")
            send(b"\x1b[<0;17;8M\x1b[<0;17;8m")
            send(b"\x1b[<0;8;3M\x1b[<0;8;3m")
            send(b"\x1b[B\x1b[B\r")
            assert not events("session.harness.changed")
            send(b"IGNORED\x1b[200~PASTE\x1b[201~")
            send(b"\x1b[A\r")
            assert events("session.harness.changed")[-1]["harness"] == "claude-code"
            # Both regular submission and deferred queue bypass native PassPort checks.
            send(b"hello", 0.2)
            send(b"\x1b[<0;3;30M\x1b[<0;3;30m", 0.4)
            output = send(b"/harness\r", 0.4)
            assert ("不能切换" if locale == "zh-CN" else "switchharnessduringaforegroundrun") in output, output
            send(b"follow-up\r", 0.4)
            drain(5)
            assert not any("IGNORED" in str(value) or "PASTE" in str(value) for value in events("user.input"))
            args = [json.loads(line) for line in log.read_text().splitlines()]
            assert len(args) >= 2, args
            assert "--resume" in args[-1] and "01a10c01-7571-71c2-9f0f-62886fc7be5b" in args[-1], args
            assert any("FAKE_REPLY" in str(value) for value in events("assistant.message.completed"))
            # Resize while the window is open; all four rows remain keyboard accessible.
            send(b"cancel-me\r", 0.4)
            send(b"/cancel\r", 0.5)
            drain(0.8)
            assert events("run.cancelled")
            send(b"/harness\r")
            for width, height in ((24, 10), (40, 24), (100, 30)):
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
                os.killpg(proc.pid, signal.SIGWINCH)
                drain()
                send(b"\x1b[B\x1b[B\x1b[A\x1b[A")
                assert proc.poll() is None
            send(b"\x1b")
            send(b"\x11")
            proc.wait(timeout=5)
            assert proc.returncode == 0
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                proc.wait(timeout=5)
            os.close(master)
    print("PASS harness PTY: zh/en, missing CLI, keys, isolation, foreground routing, queue/resume, live resize")
