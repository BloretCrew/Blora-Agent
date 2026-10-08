#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""End-to-end TUI check for ask_user, reasoning folding, /reasoning, and /plan.

Runs the real `blora` binary in a pseudo-terminal against scripts/fake_openai.py.
Build first: cargo build -p blora-cli
"""
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
import sys
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def plain(data):
    value = data.decode(errors="replace")
    value = re.sub(r"\x1b\][^\x07]*\x07", "", value)
    return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", value).replace(" ", "")


with tempfile.TemporaryDirectory(prefix="blora-ask-tui-") as directory:
    root = Path(directory)
    script = root / "script.json"
    script.write_text(json.dumps([
        {"reasoning": "SECRETTHOUGHT weighing regions", "tool": "ask_user",
         "arguments": {"question": "Which region should I deploy to?", "options": ["EU", "US"]}},
        {"text": "DEPLOYED_TO_CHOSEN_REGION"},
    ]))
    port_file = root / "port"
    server = subprocess.Popen([sys.executable, str(ROOT / "scripts/fake_openai.py"), str(port_file), str(script)])
    for _ in range(100):
        if port_file.exists() and port_file.read_text():
            break
        time.sleep(0.05)
    port = port_file.read_text()
    home = root / "home"
    workspace = root / "ws"
    workspace.mkdir()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 110, 0, 0))
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="openai", BLORA_API_KEY="test",
               BLORA_API_BASE=f"http://127.0.0.1:{port}/v1", BLORA_MODEL="fake-model",
               TERM="xterm-256color", NO_COLOR="1", BLORA_LOCALE="zh-CN",
               BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")
    proc = subprocess.Popen([str(ROOT / "target/debug/blora")], cwd=workspace, stdin=slave,
                            stdout=slave, stderr=slave, env=env, start_new_session=True)
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
        database = next(p for p in home.glob("*.sqlite*") if not p.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(database) as conn:
            return [json.loads(row[0]) for row in conn.execute(
                "SELECT payload FROM events WHERE event_type=? ORDER BY sequence", (kind,))]

    def wait_for(predicate, timeout=10, seen=""):
        deadline = time.monotonic() + timeout
        if predicate(seen):
            return seen
        while time.monotonic() < deadline:
            seen += drain(0.3)
            if predicate(seen):
                return seen
        raise AssertionError(seen[-3000:])

    try:
        first = drain(2.5)
        send(b"\x1b", 1.0)
        send(b"/provider openai\r", 0.8)
        send(b"pick a region", 0.5)
        typed = send(b"\r", 0.5)
        try:
            screen = wait_for(lambda text: "Whichregionshould" in text and "1EU" in text, seen=typed)
        except AssertionError:
            print("FIRST", first[-1500:])
            print("TYPED", typed[-1500:])
            raise
        assert "思考过程" in screen, screen[-2000:]
        assert "SECRETTHOUGHT" not in screen, "reasoning is folded by default"
        assert events("input.requested")[0]["options"] == ["EU", "US"]
        # Pick option 2 with a digit key.
        picked = send(b"2", 0.5)
        wait_for(lambda text: "DEPLOYED_TO_CHOSEN_REGION" in text, seen=picked)
        assert events("input.provided")[0]["answer"] == "US"
        requests = [json.loads(line) for line in (root / "port.requests.jsonl").read_text().splitlines()]
        tool_requests = [r for r in requests if r.get("tools")]
        last = tool_requests[-1]["messages"]
        tool_reply = [m for m in last if m["role"] == "tool"][-1]["content"]
        assert tool_reply == "User answered: US", tool_reply
        assert not any("SECRETTHOUGHT" in json.dumps(m) for m in last), "reasoning is not replayed"
        assert "# Mode: Code" in last[0]["content"]
        names = {tool["function"]["name"] for tool in tool_requests[0]["tools"]}
        assert "ask_user" in names and "web_search" not in names, names
        # /reasoning expands the folded block.
        out = send(b"/reasoning\r", 1.0)
        assert "SECRETTHOUGHT" in out or "已展开思考过程" in out, out[-1500:]
        # /plan toggles the plan permission mode.
        out = send(b"/plan\r", 0.8)
        assert "权限模式：" in out, out[-1500:]
        assert not [e for e in events("session.created") if e.get("title") == "plan"], "/plan no longer opens a session"
        print("ask-user TUI test passed")
    finally:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=5)
        server.terminate()
        server.wait(timeout=5)
        os.close(master)
