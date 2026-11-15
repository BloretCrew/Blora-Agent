# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Blora Agent contributors

import os
import unittest

from blora_agent import Agent

BINARY = os.environ.get("BLORA_BIN")


class AgentTests(unittest.TestCase):
    def test_scripted_chat(self):
        agent = Agent.start(
            binary_path=BINARY,
            profile="web",
            provider={
                "kind": "scripted",
                "completions": [{"text": "hello from python", "finish_reason": "stop"}],
            },
        )
        try:
            session = agent.create_session()
            run = session.run("hi")
            text = ""
            for event in run.events():
                if event["type"] == "assistant.delta":
                    text += event["payload"].get("text", "")
            outcome = run.wait()
            self.assertEqual(outcome["status"], "completed")
            self.assertEqual(outcome["finalText"], "hello from python")
            self.assertEqual(text, "hello from python")
        finally:
            agent.close()

    def _reentrant_tool(self, args):
        self._callback_session.project()
        return f"echo:{args.get('text', '')}"

    def test_host_tool(self):
        agent = Agent.start(
            binary_path=BINARY,
            profile="web",
            provider={
                "kind": "scripted",
                "completions": [
                    {
                        "tool_calls": [
                            {
                                "id": "call_echo",
                                "name": "echo",
                                "arguments": '{"text":"hi"}',
                            }
                        ],
                        "finish_reason": "tool_calls",
                    },
                    {"text": "done", "finish_reason": "stop"},
                ],
            },
            tools=[
                {
                    "name": "echo",
                    "description": "Echo text",
                    "parameters": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                    },
                    "read_only": True,
                    "handler": lambda args: self._reentrant_tool(args),
                }
            ],
        )
        try:
            session = agent.create_session()
            self._callback_session = session
            run = session.run("use echo")
            outcome = run.wait()
            self.assertEqual(outcome["status"], "completed")
            self.assertEqual(outcome["finalText"], "done")
        finally:
            agent.close()


if __name__ == "__main__":
    unittest.main()
