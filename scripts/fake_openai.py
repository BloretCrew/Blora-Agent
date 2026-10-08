#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Scripted OpenAI-compatible chat server for end-to-end interface tests.

Usage: fake_openai.py PORT_FILE SCRIPT_JSON

SCRIPT_JSON is a list of turns. Each turn is either {"text": "..."} or
{"tool": "name", "arguments": {...}} and optionally {"reasoning": "..."}.
Requests for session titles get a fixed title and do not consume a turn.
Every request body is appended to <PORT_FILE>.requests.jsonl.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port_file, script_path = sys.argv[1], sys.argv[2]
script = json.load(open(script_path))
state = {"turn": 0}


def chunk(delta, finish=None):
    body = {"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
    return f"data: {json.dumps(body)}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        request = json.loads(self.rfile.read(length) or b"{}")
        with open(port_file + ".requests.jsonl", "a") as log:
            log.write(json.dumps(request) + "\n")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        if not request.get("tools"):
            turn = {"text": "Fake title"}
        else:
            turn = script[min(state["turn"], len(script) - 1)]
            state["turn"] += 1
        out = []
        if turn.get("reasoning"):
            out.append(chunk({"reasoning_content": turn["reasoning"]}))
        if "tool" in turn:
            call = {"index": 0, "id": f"call_{state['turn']}", "type": "function",
                    "function": {"name": turn["tool"], "arguments": json.dumps(turn.get("arguments", {}))}}
            out.append(chunk({"tool_calls": [call]}))
            out.append(chunk({}, "tool_calls"))
        else:
            out.append(chunk({"content": turn["text"]}))
            out.append(chunk({}, "stop"))
        usage = {"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5}}
        out.append(f"data: {json.dumps(usage)}\n\n".encode())
        out.append(b"data: [DONE]\n\n")
        for piece in out:
            self.wfile.write(piece)
            self.wfile.flush()


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
with open(port_file, "w") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
