#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the right-hand file editor with the actual terminal and disk."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unicodedata

BINARY = Path("target/debug/blora").resolve()


def check(root, locale):
    project, home = root / locale / "project", root / locale / "home"
    project.mkdir(parents=True)
    source = project / "sample.py"
    original = "# 中文注释\nprint('original')\n"
    source.write_text(original)
    (project / "raw.bin").write_bytes(b"\x00\xff\x01")
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock", BLORA_LOCALE=locale,
               BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1", TERM="xterm-256color")
    socket = str(root / ("editor-" + locale))
    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)
    def screen():
        time.sleep(0.3)
        return tmux("capture-pane", "-p", "-t", "editor:0.0")
    def keys(*args):
        tmux("send-keys", "-t", "editor:0.0", *args)
        time.sleep(0.2)
    def literal(text):
        keys("-l", text)
    def click(label):
        lines = screen().splitlines()
        hits = [(i, line) for i, line in enumerate(lines) if label in line[-36:]]
        assert hits, (label, screen())
        row, line = hits[0]
        prefix = line[:line.rfind(label)]
        col = sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in prefix) + 1
        literal(f"\x1b[<0;{col};{row+1}M\x1b[<0;{col};{row+1}m")
    def wait(predicate, timeout=12):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if predicate():
                return
            time.sleep(0.15)
        raise AssertionError(screen())
    command = "env " + " ".join(shlex.quote(f"{k}={v}") for k, v in env.items())
    command += f" {shlex.quote(str(BINARY))} --workspace {shlex.quote(str(project))}"
    try:
        tmux("new-session", "-d", "-s", "editor", "-x", "160", "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        click("文件" if locale == "zh-CN" else "Files")
        wait(lambda: "sample.py" in screen())
        click("sample.py")
        wait(lambda: "original" in screen())
        assert "Python" in screen(), screen()
        # Text files open directly in the editor; disk writes require explicit save.
        keys("End")
        literal(" # EDITED_MARKER")
        assert source.read_text() == original
        keys("C-s")
        wait(lambda: "EDITED_MARKER" in source.read_text())
        saved = source.read_text()
        literal(" UNSAVED_MARKER")
        # External modifications must never be silently overwritten.
        source.write_text("# EXTERNAL_CHANGE\n")
        keys("C-s")
        time.sleep(0.5)
        assert source.read_text() == "# EXTERNAL_CHANGE\n", source.read_text()
        assert any(word in screen().lower() for word in ("外部", "冲突", "changed", "conflict")), screen()
        keys("C-g")
        keys("C-q")
        # Quit with dirty content must expose save/discard/cancel choices.
        output = screen()
        assert any(word in output.lower() for word in ("放弃", "discard", "未保存", "unsaved")), output
        print(f"PASS file editor {locale}: language, edit, explicit save, disk conflict and dirty guard")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)


with tempfile.TemporaryDirectory(prefix="blora-file-editor-") as directory:
    for language in ("zh-CN", "en"):
        check(Path(directory), language)
