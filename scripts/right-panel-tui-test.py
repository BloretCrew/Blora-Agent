#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Real-terminal checks for the right panel, PTY shell and independent chat."""
import json
import os
import shlex
import sqlite3
import subprocess
import tempfile
import time
from pathlib import Path

BINARY = Path("target/debug/blora").resolve()


def run(root, locale):
    home, project = root / locale, root / "project"
    project.mkdir(exist_ok=True)
    (project / "panel-marker.txt").write_text("FILE_PREVIEW_MARKER\n")
    (project / "directory").mkdir(exist_ok=True)
    (project / "directory" / "nested.txt").write_text("NESTED_MARKER\n")
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock",
               BLORA_LOCALE=locale, BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1",
               TERM="xterm-256color")
    subprocess.run([str(BINARY), "session", "create", "--workspace", str(project),
                    "--title", "Main session"], env=env, check=True, capture_output=True)
    socket, width = str(root / f"right-{locale}"), 160

    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)

    def screen():
        time.sleep(0.4)
        return tmux("capture-pane", "-p", "-t", "right:0.0")

    def keys(*args):
        tmux("send-keys", "-t", "right:0.0", *args)
        time.sleep(0.25)

    def literal(text):
        keys("-l", text)

    def click(label, top=False):
        lines = screen().splitlines()
        matches = [(i, line) for i, line in enumerate(lines)
                   if label in line[-36:] and (not top or i < 4)]
        assert matches, (label, screen())
        row, line = matches[0]
        # Right panel starts at column width - 36; fixed click avoids Unicode offsets.
        x = width - 32
        if top:
            prefix = line[:line.rfind(label)]
            import unicodedata
            x = sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in prefix) + 1
        literal(f"\x1b[<0;{x};{row + 1}M\x1b[<0;{x};{row + 1}m")

    def events(kind):
        database = next(path for path in home.glob("*.sqlite*") if not path.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(database) as conn:
            return list(conn.execute("SELECT session_id,payload FROM events WHERE event_type=? ORDER BY timestamp", (kind,)))

    def wait(predicate, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = screen()
            if predicate(value):
                return value
        raise AssertionError(screen())

    label = ({"info": "信息", "files": "文件", "terminal": "终端", "chat": "侧聊"}
             if locale == "zh-CN" else
             {"info": "Info", "files": "Files", "terminal": "Shell", "chat": "Side conversation"})
    command = "env " + " ".join(shlex.quote(f"{key}={value}") for key, value in env.items())
    command += f" {shlex.quote(str(BINARY))} --workspace {shlex.quote(str(project))}"
    try:
        tmux("new-session", "-d", "-s", "right", "-x", str(width), "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        assert "Main session" in screen(), screen()
        literal("/provider mock")
        keys("Enter")
        literal("MAIN_DRAFT")
        click(label["files"])
        wait(lambda text: "panel-marker.txt" in text)
        click("panel-marker.txt")
        wait(lambda text: "FILE_PREVIEW_MARKER" in text)
        keys("BSpace")
        click("directory")
        wait(lambda text: "nested.txt" in text)
        click("nested.txt")
        wait(lambda text: "NESTED_MARKER" in text)
        keys("BSpace", "BSpace")
        (project / "refreshed.txt").write_text("REFRESH_MARKER\n")
        keys("r")
        wait(lambda text: "refreshed.txt" in text)
        click(label["info"], top=True)
        click(label["terminal"])
        literal("printf 'SHELL_MARKER\\n'\r")
        wait(lambda text: "SHELL_MARKER" in text)
        literal(f"echo $$ > {shlex.quote(str(root / ('shell-' + locale)))}\r")
        wait(lambda _: (root / ("shell-" + locale)).exists())
        literal("sleep 30\r")
        time.sleep(0.5)
        keys("C-c")
        literal("printf 'AFTER_INTERRUPT\\n'\r")
        wait(lambda text: "AFTER_INTERRUPT" in text)
        literal("printf '终端中文\\n'\r")
        wait(lambda text: "终端中文" in text)
        literal("seq 1 100\r")
        wait(lambda text: "100" in text)
        keys("S-PageUp")
        history = screen()
        keys("End")
        assert screen() != history, "terminal history did not move"
        literal("exit\r")
        wait(lambda text: "退出" in text or "exit" in text.lower())
        click("重新启动" if locale == "zh-CN" else "Restart")
        literal(f"echo $$ > {shlex.quote(str(root / ('shell-' + locale)))}\r")
        time.sleep(0.5)
        keys("C-g")
        click(label["info"], top=True)
        click("侧边对话" if locale == "zh-CN" else "Side conversation")
        literal("SIDE_ONLY_MESSAGX")
        keys("Left", "Delete")
        literal("E")
        keys("Enter")
        wait(lambda _: any("SIDE_ONLY_MESSAGE" in payload for _, payload in events("user.input")))
        wait(lambda _: bool(events("assistant.message.completed")))
        rows = events("user.input")
        side = next(session for session, payload in rows if "SIDE_ONLY_MESSAGE" in payload)
        assert all("MAIN_DRAFT" not in payload for _, payload in rows), rows
        keys("C-g")
        assert "MAIN_DRAFT" in screen(), screen()
        keys(*(["BSpace"] * len("MAIN_DRAFT")))
        literal("MAIN_ONLY_MESSAGE")
        keys("Enter")
        wait(lambda _: any("MAIN_ONLY_MESSAGE" in payload for _, payload in events("user.input")))
        rows = events("user.input")
        main = next(session for session, payload in rows if "MAIN_ONLY_MESSAGE" in payload)
        assert main != side, rows
        for columns, height in [(124, 30), (123, 30), (40, 24), (24, 10), (160, 40)]:
            width = columns
            tmux("resize-window", "-t", "right", "-x", str(columns), "-y", str(height))
            screen()
        keys("C-q")
        time.sleep(1)
        pid_file = root / ("shell-" + locale)
        assert pid_file.exists(), "shell did not execute commands"
        pid = int(pid_file.read_text())
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError(f"terminal shell {pid} survived application exit")
        print(f"PASS right panel {locale}: file preview, shell input/interrupt/cleanup, independent session and drafts, resizing")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)


with tempfile.TemporaryDirectory(prefix="blora-right-panel-") as directory:
    for language in ("zh-CN", "en"):
        run(Path(directory), language)
