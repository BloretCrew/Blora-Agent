#!/usr/bin/env python3
"""Minimal LSP server for local Blora LSP testing."""

import json
import sys


def read_message():
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode("utf-8", "replace").strip()
        if not line:
            break
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1].strip())
    if length <= 0:
        return None
    body = sys.stdin.buffer.read(length)
    return json.loads(body.decode("utf-8"))


def send(message):
    body = json.dumps(message).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def result(request_id, value):
    send({"jsonrpc": "2.0", "id": request_id, "result": value})


def main():
    while True:
        message = read_message()
        if message is None:
            return
        method = message.get("method")
        request_id = message.get("id")
        params = message.get("params") or {}
        if method == "initialize":
            result(
                request_id,
                {
                    "capabilities": {
                        "textDocument": {
                            "hover": {},
                            "definition": {},
                            "references": {},
                            "documentSymbol": {},
                        },
                        "workspace": {"symbol": {}},
                    }
                },
            )
        elif method == "textDocument/didOpen":
            document = params.get("textDocument") or {}
            uri = document.get("uri", "")
            send(
                {
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": {
                        "uri": uri,
                        "diagnostics": [
                            {
                                "range": {
                                    "start": {"line": 0, "character": 0},
                                    "end": {"line": 0, "character": 1},
                                },
                                "severity": 1,
                                "message": "fake diagnostic",
                            }
                        ],
                    },
                }
            )
        elif method == "textDocument/hover":
            result(request_id, {"contents": {"kind": "markdown", "value": "fake hover"}})
        elif method == "textDocument/definition":
            result(
                request_id,
                [
                    {
                        "uri": (params.get("textDocument") or {}).get("uri", ""),
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 1},
                        },
                    }
                ],
            )
        elif method == "textDocument/references":
            result(
                request_id,
                [
                    {
                        "uri": (params.get("textDocument") or {}).get("uri", ""),
                        "range": {
                            "start": {"line": 1, "character": 2},
                            "end": {"line": 1, "character": 3},
                        },
                    }
                ],
            )
        elif method == "textDocument/documentSymbol":
            result(
                request_id,
                [
                    {
                        "name": "main",
                        "kind": 12,
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 4},
                        },
                        "selectionRange": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 4},
                        },
                    }
                ],
            )
        elif method == "workspace/symbol":
            result(
                request_id,
                [
                    {
                        "name": "main",
                        "kind": 12,
                        "location": {
                            "uri": "file:///main.rs",
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 4},
                            },
                        },
                    }
                ],
            )
        elif method == "shutdown":
            result(request_id, None)
        elif method == "exit":
            return
        elif request_id is not None:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {"code": -32601, "message": f"unknown method {method}"},
                }
            )


if __name__ == "__main__":
    main()
