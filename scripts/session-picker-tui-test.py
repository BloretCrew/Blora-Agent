#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise session search via a real PTY, without network providers."""
import codecs
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
import unicodedata
from pathlib import Path


class Screen:
    """Small VT screen reader for ratatui's cursor-addressed output."""
    def __init__(self, width, height):
        self.width, self.height = width, height
        self.cells = [[" " for _ in range(width)] for _ in range(height)]
        self.x = self.y = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, data):
        text = self.pending + self.decoder.decode(data)
        self.pending = ""
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == "\x1b":
                if i + 1 >= len(text):
                    break
                if text[i + 1] == "[":
                    match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", text[i:])
                    if not match:
                        break
                    args, _, final = match.groups()
                    nums = [int(x or 0) for x in args.split(";")] if not args.startswith(("?", ">", "<")) else []
                    n = (nums[0] or 1) if nums else 1
                    if final in "Hf":
                        self.y = max(0, (nums[0] or 1) - 1) if nums else 0
                        self.x = max(0, (nums[1] or 1) - 1) if len(nums) > 1 else 0
                    elif final == "G":
                        self.x = n - 1
                    elif final == "A":
                        self.y = max(0, self.y - n)
                    elif final == "B":
                        self.y += n
                    elif final == "C":
                        self.x += n
                    elif final == "D":
                        self.x = max(0, self.x - n)
                    elif final == "J" and nums and nums[0] in (2, 3):
                        self.cells = [[" " for _ in range(self.width)] for _ in range(self.height)]
                    elif final == "K" and self.y < self.height:
                        start = 0 if nums and nums[0] == 2 else self.x
                        for x in range(start, self.width):
                            self.cells[self.y][x] = " "
                    i += len(match.group(0))
                    continue
                if text[i + 1] == "]":
                    match = re.match(r"\x1b\].*?(?:\x07|\x1b\\)", text[i:], re.S)
                    if not match:
                        break
                    i += len(match.group(0))
                    continue
                i += 2
                continue
            if ch == "\r":
                self.x = 0
            elif ch == "\n":
                self.y = min(self.height - 1, self.y + 1)
            elif ch >= " ":
                size = 0 if unicodedata.combining(ch) else 2 if unicodedata.east_asian_width(ch) in "WF" else 1
                if self.y < self.height and self.x < self.width:
                    self.cells[self.y][self.x] = ch
                    if size == 2 and self.x + 1 < self.width:
                        self.cells[self.y][self.x + 1] = ""
                self.x += size
            i += 1
        self.pending = text[i:]

    def lines(self):
        return ["".join(row) for row in self.cells]

    def text(self):
        return "\n".join(self.lines())


with tempfile.TemporaryDirectory(prefix="blora-session-tui-") as directory:
    root = Path(directory)
    home = root / "home"
    workspace = root / "project"
    workspace.mkdir()
    binary = Path("target/debug/blora").resolve()
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock", TERM="xterm-256color",
               NO_COLOR="1", BLORA_LOCALE="zh-CN", BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")

    def cli(*args):
        return subprocess.check_output([str(binary), *args], cwd=workspace, env=env, text=True).strip()

    title_id = cli("session", "create", "--workspace", str(workspace), "--title", "标题目标Alpha")
    body_id = cli("session", "create", "--workspace", str(workspace), "--title", "正文目标Beta")
    cli("run", "--session", body_id, "--mock", "bodyneedle 中文部署")
    for n in range(18):
        cli("session", "create", "--workspace", str(workspace), "--title", f"ordinary-{n:02}")
    cli("session", "create", "--workspace", str(workspace), "--title", "CurrentSession")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    proc = subprocess.Popen([str(binary), "tui", "--workspace", str(workspace)], cwd=workspace,
                            stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    os.close(slave)
    screen = Screen(100, 30)

    def drain(duration=0.5):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.025)[0]:
                try:
                    screen.feed(os.read(master, 65536))
                except OSError:
                    break
        return screen.text()

    def send(keys, duration=0.5):
        os.write(master, keys.encode() if isinstance(keys, str) else keys)
        return drain(duration)

    def click(x, y):
        return send(f"\x1b[<0;{x + 1};{y + 1}M\x1b[<0;{x + 1};{y + 1}m")

    def resize(width, height):
        global screen
        screen = Screen(width, height)
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        os.killpg(proc.pid, signal.SIGWINCH)
        return drain(0.6)

    def expect(value):
        assert value in screen.text(), (value, screen.text())

    def open_picker():
        header = screen.lines()[0]
        # The compact layout always reserves a session entry.
        for title in ("CurrentSession", "标题目标Alpha", "正文目标Beta", "会话"):
            if title in header:
                # Map the string index back to terminal columns for wide text.
                prefix = header[:header.index(title)]
                x = sum(2 if unicodedata.east_asian_width(ch) in "WF" else 1 for ch in prefix)
                click(x, 0)
                expect("搜索")
                return
        raise AssertionError(("missing session header entry", screen.text()))

    try:
        drain(2)
        send(b"\x1b")  # Skip the first-run guide.
        drain(0.5)
        open_picker()
        send("Alpha")
        expect("标题目标Alpha")
        send(b"\r")
        expect("标题目标Alpha")
        open_picker()
        send("\x1b[200~bodyneedle\x1b[201~", 0.8)
        expect("正文目标Beta")
        expect("bodyneedle")
        send(b"\r")
        expect("正文目标Beta")
        open_picker()
        send("never-matches-xyz", 0.8)
        expect("没有匹配")
        send(b"\x7f" * len("never-matches-xyz"))
        expect("ordinary-")
        send("部署", 0.8)
        expect("正文目标Beta")
        # Fast edits must discard old title and content batches.
        send(b"\x7f\x7fAlpha", 0.8)
        expect("标题目标Alpha")
        assert "正文目标Beta" not in screen.text().split("搜索会话", 1)[-1], screen.text()
        for width, height in ((40, 24), (24, 10), (100, 30)):
            resize(width, height)
            expect("搜索")
            expect("Alpha")
            send(b"\x1b[B\x1b[A")
        send(b"\x1b")
        drain(0.3)
        # Open via the header at every supported width.
        for width, height in ((40, 24), (24, 10), (100, 30)):
            resize(width, height)
            open_picker()
            send("Alpha", 0.8)
            send(b"\r")
            if width >= 60:
                expect("标题目标Alpha")
        open_picker()
        send("bodyneedle", 0.8)
        row = next(i for i, line in enumerate(screen.lines()) if "正文目标Beta" in line)
        click(10, row)
        expect("正文目标Beta")
        database = next(path for path in home.glob("*.sqlite*") if not path.name.endswith(("-wal", "-shm")))
        with sqlite3.connect(database) as conn:
            inputs = [json.loads(row[0])["text"] for row in conn.execute("SELECT payload FROM events WHERE event_type='user.input'")]
        assert inputs == ["bodyneedle 中文部署"], inputs
        send(b"\x11", 0.3)
        proc.wait(timeout=5)
        assert proc.returncode == 0, proc.returncode
        print("PASS: session title/content search, paste, edits, navigation, mouse switch, resize and input isolation")
    finally:
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            proc.wait(timeout=5)
        os.close(master)
