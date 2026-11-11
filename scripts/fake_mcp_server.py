#!/usr/bin/env python3
"""Minimal stdio MCP server for local Blora MCP testing.

Implements tools/list, tools/call, resources/list, resources/read,
prompts/list, and prompts/get over line-delimited JSON-RPC.
"""

import json
import sys

TOOLS = [
    {
        "name": "echo",
        "description": "Echo a string back.",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
    },
    {
        "name": "add",
        "description": "Add two integers.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "a": {"type": "integer"},
                "b": {"type": "integer"},
            },
            "required": ["a", "b"],
        },
    },
]

RESOURCES = [
    {
        "uri": "fake://readme",
        "name": "readme",
        "description": "A small resource.",
        "mimeType": "text/plain",
    }
]

PROMPTS = [
    {
        "name": "greet",
        "description": "Greet someone.",
        "arguments": [{"name": "name", "required": True}],
    }
]


def reply(request_id, result):
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error(request_id, message):
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": -32000, "message": message},
    }


def handle(request):
    method = request.get("method")
    params = request.get("params") or {}
    request_id = request.get("id")
    if method == "initialize":
        return reply(
            request_id,
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}, "resources": {}, "prompts": {}},
                "serverInfo": {"name": "fake-mcp", "version": "0.1.0"},
            },
        )
    if method == "tools/list":
        return reply(request_id, {"tools": TOOLS})
    if method == "tools/call":
        name = params.get("name")
        arguments = params.get("arguments") or {}
        if name == "echo":
            return reply(
                request_id,
                {"content": [{"type": "text", "text": str(arguments.get("text", ""))}]},
            )
        if name == "add":
            total = int(arguments.get("a", 0)) + int(arguments.get("b", 0))
            return reply(
                request_id,
                {"content": [{"type": "text", "text": str(total)}]},
            )
        return error(request_id, f"unknown tool: {name}")
    if method == "resources/list":
        return reply(request_id, {"resources": RESOURCES})
    if method == "resources/read":
        if params.get("uri") == "fake://readme":
            return reply(
                request_id,
                {
                    "contents": [
                        {
                            "uri": "fake://readme",
                            "mimeType": "text/plain",
                            "text": "fake MCP resource\n",
                        }
                    ]
                },
            )
        return error(request_id, "resource not found")
    if method == "prompts/list":
        return reply(request_id, {"prompts": PROMPTS})
    if method == "prompts/get":
        name = params.get("name")
        if name != "greet":
            return error(request_id, "prompt not found")
        arguments = params.get("arguments") or {}
        who = arguments.get("name", "world")
        return reply(
            request_id,
            {
                "description": "Greeting",
                "messages": [
                    {
                        "role": "user",
                        "content": {"type": "text", "text": f"Hello, {who}!"},
                    }
                ],
            },
        )
    if method and method.startswith("notifications/"):
        return None
    return error(request_id, f"unknown method: {method}")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue
        response = handle(request)
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
