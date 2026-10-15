#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise project-tree and dual-sidebar navigation in an isolated terminal."""
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
    (workspace / "first-marker.txt").write_text("first workspace\n")
    (other / "second-marker.txt").write_text("second workspace\n")
    subprocess.run(["git", "init", "-q", "-b", "sidebar-test", str(workspace)], check=True)
    env = dict(os.environ, BLORA_HOME=str(home), BLORA_PROVIDER="mock",
               BLORA_LOCALE=locale, BLORA_TRANSLATION_BASE_URL="http://127.0.0.1:1")
    for path, title in [(other, "Other project"), (workspace, "First conversation"),
                        (workspace, "Second conversation")]:
        subprocess.run([str(BINARY), "session", "create", "--workspace", str(path),
                        "--title", title], env=env, check=True, capture_output=True)
    socket = str(root / f"tmux-{locale}")
    width = 160

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

    def click(label, side="left"):
        rows = screen().splitlines()
        start, end = (0, 27) if side == "left" else (width - 23, width)
        row = next(i for i, value in enumerate(rows) if label in value[start:end])
        column = 4 if side == "left" else width - 19
        literal(f"\x1b[<0;{column};{row + 1}M\x1b[<0;{column};{row + 1}m")

    def left_text():
        return "\n".join(line[:27] for line in screen().splitlines())

    def right_text():
        return "\n".join(line[-23:] for line in screen().splitlines())

    def resize(columns, height):
        nonlocal width
        width = columns
        tmux("resize-window", "-t", "sidebar", "-x", str(width), "-y", str(height))
        return screen()

    labels = ({"new": "新建会话", "search": "搜索会话", "projects": "项目", "settings": "设置",
               "workspace": "工作区", "files": "文件", "changes": "变更", "context": "上下文", "tasks": "任务"}
              if locale == "zh-CN" else
              {"new": "New session", "search": "Search sessions", "projects": "Projects", "settings": "Settings",
               "workspace": "Workspace", "files": "Files", "changes": "Changes", "context": "Context", "tasks": "Tasks"})
    command = "env " + " ".join(shlex.quote(f"{key}={value}") for key, value in env.items())
    command += " " + shlex.quote(str(BINARY)) + " --workspace " + shlex.quote(str(workspace))
    try:
        tmux("new-session", "-d", "-s", "sidebar", "-x", "160", "-y", "40", command)
        time.sleep(2)
        keys("Escape")
        assert labels["new"] in left_text() and labels["workspace"] in right_text(), screen()
        click("project-one")
        assert "First conversation" not in left_text(), screen()
        click("project-one")
        assert "First conversation" in left_text(), screen()
        click("First conversation")
        assert "First conversation" in screen().splitlines()[0][28:], screen()
        click("Second conversation")
        assert "Second conversation" in screen().splitlines()[0][28:], screen()
        literal("preserved-draft")
        for columns, height, left, right in [(124, 30, True, True), (123, 30, True, False),
                                            (100, 30, True, False), (99, 30, False, False),
                                            (160, 17, False, False), (160, 18, True, True),
                                            (40, 24, False, False), (24, 10, False, False),
                                            (160, 40, True, True)]:
            output = resize(columns, height)
            assert any(line.split("│")[0].strip() == labels["new"] for line in left_text().splitlines()) == left, (columns, height, output)
            assert (labels["workspace"] in right_text()) == right, (columns, height, output)
            assert "preserved-draft" in output, (columns, height, output)
        keys(*(["BSpace"] * len("preserved-draft")))
        click(labels["search"])
        literal("First conversation")
        assert "First conversation" in screen(), screen()
        keys("Enter")
        assert "First conversation" in screen().splitlines()[0][28:], screen()
        click(labels["settings"])
        assert "常规" in screen(), screen()
        click(labels["new"])
        assert "常规" in screen(), screen()
        keys("Escape")
        click("✦ Blora")
        assert not any(line.split("│")[0].strip() == labels["new"] for line in left_text().splitlines()), screen()
        top = screen().splitlines()[0]
        column = top.index("[◀]") + 2
        literal(f"\x1b[<0;{column};1M\x1b[<0;{column};1m")
        assert "First conversation" in left_text(), screen()
        click(labels["workspace"], "right")
        assert labels["workspace"] not in right_text(), screen()
        top = screen().splitlines()[0]
        column = top.index("[▶]") + 2
        literal(f"\x1b[<0;{column};1M\x1b[<0;{column};1m")
        assert labels["workspace"] in right_text(), screen()
        click(labels["files"], "right")
        assert "first-marker.txt" in screen(), screen()
        keys("Escape")
        click(labels["tasks"], "right")
        assert "no tasks" in screen().lower() or "暂无任务" in screen(), screen()
        keys("Escape")
        click(labels["context"], "right")
        assert "Context" in screen() or "上下文" in screen(), screen()
        keys("Escape")
        click(labels["changes"], "right")
        assert "first-marker.txt" in screen(), screen()
        keys("Escape")
        click("project-two")
        assert "Other project" in left_text(), screen()
        click("Other project")
        assert "project-two" in right_text(), screen()
        click(labels["files"], "right")
        assert "second-marker.txt" in screen() and "first-marker.txt" not in screen(), screen()
        keys("Escape")
        click(labels["new"])
        assert "Other project" in left_text(), screen()
        keys("C-q")
        print(f"PASS dual sidebar PTY {locale}: project tree, navigation, workspace actions, modal isolation, responsive draft")
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], capture_output=True)


with tempfile.TemporaryDirectory(prefix="blora-sidebar-tui-") as directory:
    for locale in ("zh-CN", "en"):
        check_locale(Path(directory), locale)
