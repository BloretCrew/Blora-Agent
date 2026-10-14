#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise adaptive sidebar navigation in an isolated tmux terminal."""
import os
import shlex
import subprocess
import tempfile
import time
from pathlib import Path

BINARY = Path("target/debug/blora").resolve()


def check_locale(root, locale):
    home = root / locale
    workspace = root / "project-one"
    other = root / "project-two"
    workspace.mkdir(exist_ok=True)
    other.mkdir(exist_ok=True)
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock",
               BLORA_LOCALE=locale, BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")
    for path, title in [(other, "Other project"), (workspace, "First conversation"),
                        (workspace, "Second conversation")]:
        subprocess.run([str(BINARY), "session", "create", "--workspace", str(path),
                        "--title", title], env=env, check=True, capture_output=True)
    socket = str(root / f"tmux-{locale}")

    def tmux(*args):
        return subprocess.check_output(["tmux", "-f", "/dev/null", "-S", socket, *args], text=True)

    def screen():
        time.sleep(0.35)
        return tmux("capture-pane", "-p", "-t", "sidebar:0.0")

    def keys(*values):
        tmux("send-keys", "-t", "sidebar:0.0", *values)
        time.sleep(0.3)

    def literal(value):
        keys("-l", value)

    def click(label):
        rows = screen().splitlines()
        row = next(i for i, value in enumerate(rows) if label in value[:27])
        literal(f"\x1b[<0;4;{row + 1}M\x1b[<0;4;{row + 1}m")

    def resize(width, height):
        tmux("resize-window", "-t", "sidebar", "-x", str(width), "-y", str(height))
        return screen()

    labels = ({"new": "新建会话", "search": "搜索会话", "recent": "最近会话", "settings": "设置"}
              if locale == "zh-CN" else
              {"new": "New session", "search": "Search sessions", "recent": "Recent sessions", "settings": "Settings"})
    command = "env " + " ".join(shlex.quote(f"{key}={value}") for key, value in env.items())
    command += " " + shlex.quote(str(BINARY)) + " --workspace " + shlex.quote(str(workspace))
    try:
        tmux("new-session", "-d", "-s", "sidebar", "-x", "120", "-y", "30", command)
        time.sleep(2)
        keys("Escape")
        output = screen()
        assert labels["new"] in output and labels["recent"] in output, output
        click("First conversation")
        assert "First conversation" in screen().splitlines()[0][28:], screen()
        click("Second conversation")
        assert "Second conversation" in screen().splitlines()[0][28:], screen()
        literal("preserved-draft")
        for width, height, visible in [(99, 30, False), (100, 30, True),
                                       (120, 17, False), (120, 18, True),
                                       (40, 24, False), (24, 10, False), (120, 30, True)]:
            output = resize(width, height)
            assert (labels["recent"] in output) == visible, (width, height, output)
            assert "preserved-draft" in output, (width, height, output)
        keys(*(["BSpace"] * len("preserved-draft")))
        click(labels["search"])
        literal("First conversation")
        output = screen()
        assert "First conversation" in output, output
        keys("Enter")
        assert "First conversation" in screen().splitlines()[0][28:], screen()
        click(labels["settings"])
        output = screen()
        assert "常规" in output, output
        # Clicking the sidebar under a modal keeps the settings window active.
        click(labels["new"])
        assert "常规" in screen(), screen()
        keys("Escape")
        click("project-two")
        output = screen()
        assert "Other project" in output and "First conversation" not in output[:output.find("❯")], output
        click(labels["new"])
        output = screen()
        assert "Other project" in output, output
        keys("C-q")
        print(f"PASS sidebar PTY {locale}: sessions, search, projects, new, settings, modal isolation, responsive draft")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)


with tempfile.TemporaryDirectory(prefix="blora-sidebar-tui-") as directory:
    for locale in ("zh-CN", "en"):
        check_locale(Path(directory), locale)
