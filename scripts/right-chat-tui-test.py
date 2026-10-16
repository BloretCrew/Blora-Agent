#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Verify side-session question and approval ownership against a local provider."""
import json
import os
from pathlib import Path
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="blora-side-chat-") as directory:
    root = Path(directory)
    project = root / "project"
    project.mkdir()
    home, port, script = root / "home", root / "port", root / "turns.json"
    script.write_text(json.dumps([
        {"tool": "ask_user", "arguments": {"question": "SIDE_QUESTION", "options": ["EU", "US"]}},
        {"tool": "write_file", "arguments": {"path": "side-approved.txt", "contents": "SIDE_APPROVED"}},
        {"text": "SIDE_COMPLETED"},
        {"text": "MAIN_COMPLETED"},
    ]))
    server = subprocess.Popen([sys.executable, str(ROOT / "scripts/fake_openai.py"), str(port), str(script)])
    socket = str(root / "tmux")
    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)
    def screen():
        time.sleep(0.3)
        return tmux("capture-pane", "-p", "-t", "chat:0.0")
    def keys(*args):
        tmux("send-keys", "-t", "chat:0.0", *args)
        time.sleep(0.3)
    def literal(text):
        keys("-l", text)
    def wait(predicate, timeout=15):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if predicate():
                return
            time.sleep(0.15)
        raise AssertionError(screen())
    def events(kind):
        db = next(p for p in home.glob("*.sqlite*") if not p.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(db) as connection:
            return [(sid, json.loads(payload)) for sid, payload in connection.execute(
                "SELECT session_id,payload FROM events WHERE event_type=? ORDER BY timestamp", (kind,))]
    try:
        wait(lambda: port.exists())
        env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="openai", BLORA_API_KEY="test",
                   BLORA_API_BASE=f"http://127.0.0.1:{port.read_text()}/v1", BLORA_MODEL="fake-model",
                   BLORA_LOCALE="zh-CN", BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")
        command = "env " + " ".join(shlex.quote(f"{k}={v}") for k, v in env.items())
        command += f" {ROOT / 'target/debug/blora'} --workspace {shlex.quote(str(project))}"
        tmux("new-session", "-d", "-s", "chat", "-x", "180", "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        literal("/provider openai")
        keys("Enter")
        literal("MAIN_DRAFT")
        rows = screen().splitlines()
        row = next(i for i, line in enumerate(rows) if "侧边对话" in line[-36:])
        literal(f"\x1b[<0;150;{row+1}M\x1b[<0;150;{row+1}m")
        literal("SIDE_REQUEST")
        keys("Enter")
        wait(lambda: "SIDE_QUESTION" in screen())
        assert "Fake title" in screen(), screen()
        keys("Space", "Enter")
        wait(lambda: bool(events("approval.requested")))
        wait(lambda: "y allow" in screen())
        time.sleep(0.3)
        keys("y")
        wait(lambda: (project / "side-approved.txt").exists())
        wait(lambda: bool(events("assistant.message.completed")))
        question_owner = events("input.requested")[0][0]
        assert events("input.provided")[0][0] == question_owner
        assert events("approval.requested")[0][0] == question_owner
        assert (project / "side-approved.txt").read_text() == "SIDE_APPROVED"
        db = next(p for p in home.glob("*.sqlite*") if not p.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(db) as connection:
            title = connection.execute("SELECT title FROM sessions WHERE id=?", (question_owner,)).fetchone()[0]
        assert title and title != "侧边对话", title
        assert title in screen(), screen()
        keys("BTab")
        wait(lambda: "权限模式" in screen() and "侧聊" in screen())
        keys("Down", "Enter")
        time.sleep(0.5)
        assert "自动接受编辑" in screen(), screen()
        rows = screen().splitlines()
        control_row = next(i for i,line in enumerate(rows) if "自动接受编辑" in line[-36:] and "openai" in line[-36:])
        literal(f"\x1b[<0;174;{control_row+1}M\x1b[<0;174;{control_row+1}m")
        wait(lambda: "供应商" in screen() and "侧聊" in screen())
        keys("Escape")
        time.sleep(0.5)
        keys("C-g")
        assert "MAIN_DRAFT" in screen(), screen()
        assert "默认确认" in screen(), screen()
        keys(*(["BSpace"] * len("MAIN_DRAFT")))
        literal("MAIN_REQUEST")
        keys("Enter")
        wait(lambda: any("MAIN_COMPLETED" in str(payload) for _, payload in events("assistant.message.completed")))
        main = next(sid for sid, payload in events("user.input") if "MAIN_REQUEST" in str(payload))
        assert main != question_owner
        keys("C-q")
        print("PASS side chat: independent question, approval, write, completion, main draft and session ownership")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)
        server.terminate()
        server.wait(timeout=5)
