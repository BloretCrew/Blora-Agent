# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Blora Agent contributors

from __future__ import annotations

import json
import os
import queue
import shutil
import subprocess
import threading
from typing import Any, Callable, Iterator, Optional

_END = object()


class BloraError(Exception):
    """Protocol or runtime error."""

    def __init__(self, message: str, data: Any = None) -> None:
        super().__init__(message)
        self.data = data


def _binary_from(options: dict[str, Any]) -> str:
    explicit = options.get("binary_path")
    if explicit:
        return explicit
    env = os.environ.get("BLORA_BIN")
    if env:
        return env
    found = shutil.which("blora")
    if found:
        return found
    raise BloraError("blora binary not found; set BLORA_BIN or binary_path")


def _normalize_tool_result(value: Any) -> dict[str, Any]:
    if isinstance(value, str):
        return {"parts": [{"type": "text", "text": value}], "metadata": {}}
    if isinstance(value, dict) and isinstance(value.get("parts"), list):
        return value
    return {"parts": [{"type": "text", "text": json.dumps(value)}], "metadata": {}}


class Run:
    """One agent run."""

    def __init__(self, agent: "Agent", run_id: str, start_sequence: int) -> None:
        self.agent = agent
        self.id = run_id
        self.start_sequence = start_sequence
        self.session_id: Optional[str] = None
        self._queue: queue.Queue[Any] = queue.Queue()
        self._ended = False
        self._done: Optional[dict[str, Any]] = None

    def _push(self, event: dict[str, Any]) -> None:
        if not self._ended:
            self._queue.put(event)

    def _end(self, outcome: Optional[dict[str, Any]] = None) -> None:
        if self._ended:
            return
        self._ended = True
        if outcome is not None:
            self._done = outcome
        self._queue.put(_END)

    def events(self) -> Iterator[dict[str, Any]]:
        while True:
            event = self._queue.get()
            if event is _END:
                return
            yield event

    def wait(self, timeout: Optional[float] = None) -> dict[str, Any]:
        if self._done is not None:
            return self._done
        outcome = self.agent._request(
            "run.wait",
            {
                "runId": self.id,
                "timeoutMs": int((timeout if timeout is not None else 300) * 1000),
            },
        )
        self._done = outcome
        self._end(outcome)
        return outcome

    def cancel(self) -> None:
        self.agent._request("run.cancel", {"runId": self.id})

    def steer(self, message: str) -> None:
        if not self.session_id:
            raise BloraError("run is not attached to a session")
        self.agent._request("run.steer", {"sessionId": self.session_id, "message": message})


class Session:
    """A durable session within one agent."""

    def __init__(self, agent: "Agent", session_id: str) -> None:
        self.agent = agent
        self.id = session_id

    def run(self, prompt: str, **options: Any) -> Run:
        payload = {
            "sessionId": self.id,
            "prompt": prompt,
            "model": options.get("model"),
            "maxTurns": options.get("max_turns"),
            "permission": options.get("permission"),
            "outputSchema": options.get("output_schema"),
            "attachments": options.get("attachments"),
        }
        payload = {key: value for key, value in payload.items() if value is not None}
        started = self.agent._request("run.start", payload)
        run = Run(self.agent, started["runId"], started.get("startSequence", 0))
        run.session_id = self.id
        self.agent._adopt_run(run)
        return run

    def events(self, after_sequence: int = 0) -> list[dict[str, Any]]:
        return self.agent._request(
            "session.events",
            {"sessionId": self.id, "afterSequence": after_sequence},
        )["events"]

    def project(self) -> dict[str, Any]:
        return self.agent._request("session.project", {"sessionId": self.id})

    def resolve_approval(self, approval_id: str, allow: bool) -> None:
        self.agent._request(
            "approval.resolve",
            {
                "sessionId": self.id,
                "approvalId": approval_id,
                "decision": "allow" if allow else "deny",
            },
        )

    def answer_input(self, request_id: str, answer: Optional[str]) -> None:
        self.agent._request(
            "input.answer",
            {"sessionId": self.id, "requestId": request_id, "answer": answer},
        )

    def steer(self, message: str) -> None:
        self.agent._request("run.steer", {"sessionId": self.id, "message": message})


class Agent:
    """Embedded agent process."""

    @classmethod
    def start(cls, **options: Any) -> "Agent":
        binary = _binary_from(options)
        command = [binary, "sdk", "--stdio"]
        if options.get("allow_test_provider") or (
            isinstance(options.get("provider"), dict)
            and options["provider"].get("kind") == "scripted"
        ):
            command.append("--allow-test-provider")
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None,
            text=True,
            bufsize=1,
        )
        agent = cls(process, options)
        agent._initialize()
        return agent

    def __init__(self, process: subprocess.Popen[str], options: dict[str, Any]) -> None:
        self._process = process
        self._options = options
        self._pending: dict[Any, queue.Queue[Any]] = {}
        self._next_id = 1
        self._lock = threading.Lock()
        self._closed = False
        self._runs: dict[str, Run] = {}
        self._buffered: dict[str, list[dict[str, Any]]] = {}
        self._finished_early: dict[str, dict[str, Any]] = {}
        self._subscriptions: dict[str, dict[str, Any]] = {}
        self._dispatch: queue.Queue[Any] = queue.Queue()
        self._dispatcher = threading.Thread(target=self._dispatch_loop, daemon=True)
        self._dispatcher.start()
        self._tools = {
            tool["name"]: tool for tool in options.get("tools", []) if "name" in tool
        }
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    # -- lifecycle -----------------------------------------------------

    def __enter__(self) -> "Agent":
        return self

    def __exit__(self, *_exc: Any) -> None:
        self.close()

    def _initialize(self) -> dict[str, Any]:
        tools = [
            {
                "name": tool["name"],
                "description": tool.get("description", ""),
                "parameters": tool.get(
                    "parameters", {"type": "object", "properties": {}}
                ),
                "readOnly": tool.get("read_only", False),
                "requiresApproval": tool.get(
                    "requires_approval", not tool.get("read_only", False)
                ),
            }
            for tool in self._options.get("tools", [])
        ]
        return self._request(
            "initialize",
            {
                "protocolVersion": "1.0",
                "profile": self._options.get("profile", "full"),
                "persistence": self._options.get("persistence", {"kind": "memory"}),
                "provider": self._options.get("provider", {"kind": "mock"}),
                "workspace": self._options.get("workspace"),
                "callbacks": self._options.get("callbacks"),
                **({"search": self._options["search"]} if self._options.get("search") is not None else {}),
                **({"fetch": self._options["fetch"]} if self._options.get("fetch") is not None else {}),
                "hostTools": tools,
            },
        )

    def close(self) -> None:
        if self._closed:
            return
        try:
            self._request("agent.shutdown", {"cancelRunning": True, "timeoutMs": 5000})
        except Exception:
            pass
        self._closed = True
        try:
            self._process.stdin.close()
        except Exception:
            pass
        try:
            self._process.wait(timeout=6)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait(timeout=2)
        self._dispatch.put(_END)
        for subscription in self._subscriptions.values():
            subscription["queue"].put(_END)

    # -- sessions ------------------------------------------------------

    def create_session(self, **spec: Any) -> Session:
        result = self._request("session.create", spec)
        return Session(self, result["sessionId"])

    def createSession(self, **spec: Any) -> Session:  # noqa: N802
        return self.create_session(**spec)

    def open_session(self, session_id: str) -> Session:
        self._request("session.project", {"sessionId": session_id})
        return Session(self, session_id)

    def list_sessions(self) -> list[dict[str, Any]]:
        return self._request("session.list")["sessions"]

    # -- transport -----------------------------------------------------

    def _request(self, method: str, params: Optional[dict[str, Any]] = None) -> Any:
        if self._closed:
            raise BloraError("agent is closed")
        with self._lock:
            request_id = self._next_id
            self._next_id += 1
            waiter: queue.Queue[Any] = queue.Queue()
            self._pending[request_id] = waiter
        try:
            self._send({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params or {},
            })
            try:
                result = waiter.get(timeout=310)
            except queue.Empty as error:
                raise BloraError(f"request {method} timed out") from error
        except Exception:
            with self._lock:
                self._pending.pop(request_id, None)
            raise
        finally:
            with self._lock:
                self._pending.pop(request_id, None)
        if isinstance(result, dict) and result.get("__error__"):
            error = result["__error__"]
            raise BloraError(error.get("message", "request failed"), error.get("data"))
        return result

    def _send(self, value: dict[str, Any]) -> None:
        assert self._process.stdin is not None
        self._process.stdin.write(json.dumps(value) + "\n")
        self._process.stdin.flush()

    def _dispatch_loop(self) -> None:
        while True:
            message = self._dispatch.get()
            if message is _END:
                return
            self._handle_message(message)

    def subscribe(self, session_id: str, after_sequence: int = 0) -> Iterator[dict[str, Any]]:
        result = self._request(
            "session.subscribe",
            {"sessionId": session_id, "afterSequence": after_sequence},
        )
        subscription_id = result["subscriptionId"]
        subscription = {"queue": queue.Queue(), "session_id": session_id}
        self._subscriptions[subscription_id] = subscription
        try:
            while True:
                event = subscription["queue"].get()
                if event is _END:
                    return
                yield event
        finally:
            self._subscriptions.pop(subscription_id, None)
            try:
                self._request("session.unsubscribe", {"subscriptionId": subscription_id})
            except Exception:
                pass

    def archive_session(self, session_id: str) -> None:
        self._request("session.archive", {"sessionId": session_id})

    def _read_loop(self) -> None:
        assert self._process.stdout is not None
        for line in self._process.stdout:
            if not line.strip():
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "method" in message:
                self._dispatch.put(message)
                continue
            with self._lock:
                waiter = self._pending.get(message.get("id"))
            if waiter is None:
                continue
            if message.get("error"):
                waiter.put({"__error__": message["error"]})
            else:
                waiter.put(message.get("result"))

    def _handle_message(self, message: dict[str, Any]) -> None:
        method = message["method"]
        params = message.get("params") or {}
        if method == "event":
            event = params["event"]
            for subscription in self._subscriptions.values():
                if subscription["session_id"] == event.get("session_id"):
                    subscription["queue"].put(event)
            run = self._runs.get(event.get("run_id"))
            if run is not None:
                run._push(event)
            else:
                self._buffered.setdefault(event.get("run_id"), []).append(event)
        elif method == "run.finished":
            run = self._runs.get(params.get("runId"))
            if run is not None:
                run._end(params)
            else:
                self._finished_early[params.get("runId")] = params
            self._runs.pop(params.get("runId"), None)
        elif method == "stream.lagged":
            handler = self._options.get("on_stream_lagged")
            if handler is not None:
                threading.Thread(target=handler, args=(params,), daemon=True).start()
        elif method == "approval.requested":
            threading.Thread(
                target=self._handle_approval, args=(params,), daemon=True
            ).start()
        elif method == "input.requested":
            threading.Thread(
                target=self._handle_question, args=(params,), daemon=True
            ).start()
        elif method == "host_tool.invoke":
            threading.Thread(
                target=self._handle_host_tool, args=(message,), daemon=True
            ).start()

    def _handle_approval(self, params: dict[str, Any]) -> None:
        allow = False
        handler = self._options.get("on_approval")
        if handler is not None:
            try:
                decision = handler(params)
                allow = decision is True or decision == "allow"
            except Exception:
                allow = False
        try:
            self._request(
                "approval.resolve",
                {
                    "sessionId": params.get("sessionId"),
                    "approvalId": params.get("approvalId"),
                    "decision": "allow" if allow else "deny",
                },
            )
        except Exception:
            pass

    def _handle_question(self, params: dict[str, Any]) -> None:
        answer = None
        handler = self._options.get("on_question")
        if handler is not None:
            try:
                answer = handler(params)
            except Exception:
                answer = None
        try:
            self._request(
                "input.answer",
                {
                    "sessionId": params.get("sessionId"),
                    "requestId": params.get("requestId"),
                    "answer": answer,
                },
            )
        except Exception:
            pass

    def _handle_host_tool(self, message: dict[str, Any]) -> None:
        params = message.get("params") or {}
        tool = self._tools.get(params.get("tool"))
        handler: Optional[Callable[[dict[str, Any]], Any]] = (
            tool.get("handler") if tool else None
        )
        if handler is None:
            self._send(
                {
                    "jsonrpc": "2.0",
                    "id": message.get("id"),
                    "error": {
                        "code": -32010,
                        "message": f"host tool {params.get('tool')} has no handler",
                        "data": {"kind": "host_tool_failed"},
                    },
                }
            )
            return
        try:
            result = handler(params.get("arguments") or {})
            self._send(
                {
                    "jsonrpc": "2.0",
                    "id": message.get("id"),
                    "result": _normalize_tool_result(result),
                }
            )
        except Exception as error:  # noqa: BLE001
            self._send(
                {
                    "jsonrpc": "2.0",
                    "id": message.get("id"),
                    "error": {
                        "code": -32010,
                        "message": str(error),
                        "data": {"kind": "host_tool_failed"},
                    },
                }
            )

    def _adopt_run(self, run: Run) -> None:
        self._runs[run.id] = run
        for event in self._buffered.pop(run.id, []):
            run._push(event)
        finished = self._finished_early.pop(run.id, None)
        if finished is not None:
            run._end(finished)
