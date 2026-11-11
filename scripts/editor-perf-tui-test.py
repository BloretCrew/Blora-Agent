#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Smoke test: a large editable file stays responsive in the right panel.

Developer-run only (debug builds are slow); the threshold is deliberately
generous and exists to catch a regression back to whole-file highlighting.
"""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unicodedata

BINARY = Path("target/debug/blora").resolve()
LINES = 30_000


def check(root):
    project, home = root / "project", root / "home"
    project.mkdir(parents=True)
    big = project / "big.rs"
    big.write_text("\n".join(f"let value_{index} = {index};" for index in range(LINES)) + "\n")
    env = dict(
        os.environ,
        BLORA_HOME=str(home),
        BLORA_PROVIDER="mock",
        BLORA_LOCALE="en",
        BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1",
        TERM="xterm-256color",
    )
    socket = str(root / "perf")
    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)
    def screen():
        return tmux("capture-pane", "-p", "-t", "perf:0.0")
    def keys(*args):
        tmux("send-keys", "-t", "perf:0.0", *args)
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
    def wait(predicate, timeout=20):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if predicate():
                return True
            time.sleep(0.1)
        return False
    command = "env " + " ".join(shlex.quote(f"{k}={v}") for k, v in env.items())
    command += f" {shlex.quote(str(BINARY))} --workspace {shlex.quote(str(project))}"
    try:
        tmux("new-session", "-d", "-s", "perf", "-x", "160", "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        click("Files")
        assert wait(lambda: "big.rs" in screen()), screen()
        click("big.rs")
        assert wait(lambda: "value_0" in screen()), screen()
        started = time.monotonic()
        keys("C-End")
        literal(" // PERF_MARKER")
        assert wait(lambda: "PERF_MARKER" in screen(), timeout=20), screen()
        elapsed = time.monotonic() - started
        keys("C-s")
        assert wait(lambda: "PERF_MARKER" in big.read_text(), timeout=20), screen()
        assert elapsed < 10.0, f"large-file edit took {elapsed:.1f}s"
        # Jumping back to the top must also stay bounded.
        keys("C-Home")
        assert wait(lambda: "value_0" in screen()), screen()
        print(f"PASS large editor: {LINES} lines, Ctrl+End edit in {elapsed:.2f}s")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)


with tempfile.TemporaryDirectory(prefix="blora-editor-perf-") as directory:
    check(Path(directory))
