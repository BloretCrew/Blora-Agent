#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Verify @ session selection and actual reference content sent to the model."""
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
with tempfile.TemporaryDirectory(prefix="blora-session-mentions-") as directory:
    root = Path(directory)
    project = root / "project"
    project.mkdir()
    home, port, script = root / "home", root / "port", root / "turns.json"
    script.write_text(json.dumps([{"text": "REFERENCE_REPLY"}]))
    server = subprocess.Popen([sys.executable, str(ROOT / "scripts/fake_openai.py"), str(port), str(script)])
    socket = str(root / "tmux")
    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)
    def screen():
        time.sleep(0.3)
        return tmux("capture-pane", "-p", "-t", "mentions:0.0")
    def keys(*args):
        tmux("send-keys", "-t", "mentions:0.0", *args)
        time.sleep(0.25)
    def literal(text):
        keys("-l", text)
    def wait(predicate, timeout=15):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if predicate(): return
            time.sleep(0.15)
        raise AssertionError(screen())
    def events(kind):
        db = next(p for p in home.glob("*.sqlite*") if not p.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(db) as conn:
            return [(sid, json.loads(payload)) for sid, payload in conn.execute(
                "SELECT session_id,payload FROM events WHERE event_type=? ORDER BY timestamp", (kind,))]
    def requests():
        file = root / "port.requests.jsonl"
        return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []
    try:
        wait(lambda: port.exists())
        env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="openai", BLORA_API_KEY="test",
                   BLORA_API_BASE=f"http://127.0.0.1:{port.read_text()}/v1", BLORA_MODEL="fake-model",
                   BLORA_LOCALE="zh-CN", BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")
        for title in ("Reference source", "Mention main"):
            subprocess.run([str(ROOT / "target/debug/blora"), "session", "create", "--workspace", str(project), "--title", title], env=env, check=True, capture_output=True)
        command = "env " + " ".join(shlex.quote(f"{k}={v}") for k, v in env.items())
        command += f" {ROOT / 'target/debug/blora'} --workspace {shlex.quote(str(project))}"
        tmux("new-session", "-d", "-s", "mentions", "-x", "180", "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        rows = screen().splitlines()
        row = next(i for i,line in enumerate(rows) if "Reference source" in line[:27])
        literal(f"\x1b[<0;4;{row+1}M\x1b[<0;4;{row+1}m")
        literal("/provider openai")
        keys("Enter")
        literal("SOURCE_SECRET_FACT_9472")
        keys("Enter")
        wait(lambda: bool(events("assistant.message.completed")))
        source = next(sid for sid,payload in events("user.input") if "SOURCE_SECRET_FACT_9472" in str(payload))
        rows = screen().splitlines()
        row = next(i for i,line in enumerate(rows) if "Mention main" in line[:27])
        literal(f"\x1b[<0;4;{row+1}M\x1b[<0;4;{row+1}m")
        literal("/provider openai")
        keys("Enter")
        literal("@Reference")
        wait(lambda: "Reference source" in screen())
        keys("Tab")
        literal("MAIN_REFERENCE_REQUEST")
        keys("Enter")
        wait(lambda: any("MAIN_REFERENCE_REQUEST" in str(p) for _,p in events("user.input")))
        wait(lambda: any("MAIN_REFERENCE_REQUEST" in json.dumps(r) and "SOURCE_SECRET_FACT_9472" in json.dumps(r) for r in requests() if r.get("tools")))
        rows = screen().splitlines()
        row = next(i for i,line in enumerate(rows) if "侧边对话" in line[-36:])
        literal(f"\x1b[<0;150;{row+1}M\x1b[<0;150;{row+1}m")
        literal("@Reference")
        keys("Tab")
        literal("SIDE_REFERENCE_REQUEST")
        keys("Enter")
        wait(lambda: any("SIDE_REFERENCE_REQUEST" in json.dumps(r) and "SOURCE_SECRET_FACT_9472" in json.dumps(r) for r in requests() if r.get("tools")))
        main = next(sid for sid,p in events("user.input") if "MAIN_REFERENCE_REQUEST" in str(p))
        side = next(sid for sid,p in events("user.input") if "SIDE_REFERENCE_REQUEST" in str(p))
        assert len({source, main, side}) == 3
        keys("C-q")
        print("PASS session mentions: main/side picker, stable reference, provider receives source transcript, independent sessions")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)
        server.terminate()
        server.wait(timeout=5)
