// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const END = Symbol("end");

export class BloraError extends Error {
  constructor(message, data) {
    super(message);
    this.name = "BloraError";
    this.data = data;
  }
}

function binaryFrom(options) {
  return options.binaryPath || process.env.BLORA_BIN || "blora";
}

function normalizeToolResult(value) {
  if (typeof value === "string") {
    return { parts: [{ type: "text", text: value }], metadata: {} };
  }
  if (value && Array.isArray(value.parts)) {
    return value;
  }
  return { parts: [{ type: "text", text: JSON.stringify(value) }], metadata: {} };
}

export class Run {
  constructor(agent, id, startSequence) {
    this.agent = agent;
    this.id = id;
    this.startSequence = startSequence;
    this.queue = [];
    this.waiters = [];
    this.ended = false;
    this.done = null;
  }

  push(event) {
    const waiter = this.waiters.shift();
    if (waiter) {
      waiter(event);
    } else {
      this.queue.push(event);
    }
  }

  end(outcome) {
    if (this.ended) return;
    this.ended = true;
    this.done = outcome ?? this.done;
    for (const waiter of this.waiters.splice(0)) {
      waiter(END);
    }
  }

  nextEvent() {
    if (this.queue.length > 0) {
      return Promise.resolve(this.queue.shift());
    }
    if (this.ended) {
      return Promise.resolve(END);
    }
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  async *events() {
    while (true) {
      const event = await this.nextEvent();
      if (event === END) return;
      yield event;
    }
  }

  async wait(options = {}) {
    if (this.done) return this.done;
    const outcome = await this.agent.request("run.wait", {
      runId: this.id,
      timeoutMs: options.timeoutMs ?? 300000,
    });
    this.done = outcome;
    this.end(outcome);
    return outcome;
  }

  async cancel() {
    await this.agent.request("run.cancel", { runId: this.id });
  }

  async steer(message) {
    if (!this.sessionId) throw new BloraError("run is not attached to a session");
    await this.agent.request("run.steer", { sessionId: this.sessionId, message });
  }
}

export class Session {
  constructor(agent, id) {
    this.agent = agent;
    this.id = id;
  }

  async run(prompt, options = {}) {
    const started = await this.agent.request("run.start", {
      sessionId: this.id,
      prompt,
      model: options.model,
      maxTurns: options.maxTurns,
      permission: options.permission,
      outputSchema: options.outputSchema,
      attachments: options.attachments,
    });
    const run = new Run(this.agent, started.runId, started.startSequence ?? 0);
    run.sessionId = this.id;
    this.agent.adoptRun(run);
    return run;
  }

  async events(afterSequence = 0) {
    const result = await this.agent.request("session.events", {
      sessionId: this.id,
      afterSequence,
    });
    return result.events;
  }

  async project() {
    return this.agent.request("session.project", { sessionId: this.id });
  }

  async resolveApproval(approvalId, allow) {
    await this.agent.request("approval.resolve", {
      sessionId: this.id,
      approvalId,
      decision: allow ? "allow" : "deny",
    });
  }

  async answerInput(requestId, answer) {
    await this.agent.request("input.answer", {
      sessionId: this.id,
      requestId,
      answer,
    });
  }

  async steer(message) {
    await this.agent.request("run.steer", { sessionId: this.id, message });
  }
}

export class Agent {
  static async start(options = {}) {
    const command = ["sdk", "--stdio"];
    if (options.allowTestProvider || options.provider?.kind === "scripted") {
      command.push("--allow-test-provider");
    }
    const child = spawn(binaryFrom(options), command, {
      stdio: ["pipe", "pipe", "inherit"],
    });
    const agent = new Agent(child, options);
    await agent.initialize();
    return agent;
  }

  constructor(child, options) {
    this.child = child;
    this.options = options;
    this.nextId = 1;
    this.pending = new Map();
    this.callbackQueue = [];
    this.callbackRunning = false;
    this.runs = new Map();
    this.buffered = new Map();
    this.finishedEarly = new Map();
    this.subscriptions = new Map();
    this.closed = false;
    this.tools = new Map((options.tools ?? []).map((tool) => [tool.name, tool]));

    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      if (!line.trim()) return;
      try {
        this.onMessage(JSON.parse(line));
      } catch (error) {
        this.options.onError?.(error);
      }
    });
    child.on("exit", () => {
      this.closed = true;
      for (const { reject } of this.pending.values()) {
        reject(new BloraError("blora sdk exited"));
      }
      this.pending.clear();
      for (const run of this.runs.values()) run.end();
      for (const subscription of this.subscriptions.values()) subscription.end();
    });
  }

  async initialize() {
    return this.request("initialize", {
      protocolVersion: "1.0",
      profile: this.options.profile ?? "full",
      persistence: this.options.persistence ?? { kind: "memory" },
      provider: this.options.provider ?? { kind: "mock" },
      workspace: this.options.workspace,
      callbacks: this.options.callbacks,
      ...(this.options.search ? { search: this.options.search } : {}),
      ...(this.options.fetch ? { fetch: this.options.fetch } : {}),
      hostTools: (this.options.tools ?? []).map((tool) => ({
        name: tool.name,
        description: tool.description ?? "",
        parameters: tool.parameters ?? { type: "object", properties: {} },
        readOnly: tool.readOnly ?? false,
        requiresApproval: tool.requiresApproval ?? !(tool.readOnly ?? false),
      })),
    });
  }

  request(method, params = {}) {
    if (this.closed) {
      return Promise.reject(new BloraError("agent is closed"));
    }
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new BloraError(`request ${method} timed out`));
      }, this.options.requestTimeoutMs ?? 310000);
      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value); },
        reject: (error) => { clearTimeout(timer); reject(error); },
      });
      try {
        this.send({ jsonrpc: "2.0", id, method, params });
      } catch (error) {
        this.pending.delete(id);
        clearTimeout(timer);
        reject(error);
      }
    });
  }

  send(value) {
    this.child.stdin.write(`${JSON.stringify(value)}\n`);
  }

  onMessage(message) {
    if (message.method) {
      this.onNotification(message);
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new BloraError(message.error.message, message.error.data));
    } else {
      pending.resolve(message.result);
    }
  }

  onNotification(message) {
    const params = message.params ?? {};
    switch (message.method) {
      case "event": {
        const event = params.event;
        for (const subscription of this.subscriptions.values()) {
          if (subscription.sessionId === event.session_id) subscription.queue.push(event);
        }
        const run = this.runs.get(event.run_id);
        if (run) {
          run.push(event);
        } else {
          const list = this.buffered.get(event.run_id) ?? [];
          list.push(event);
          this.buffered.set(event.run_id, list);
        }
        break;
      }
      case "run.finished": {
        const run = this.runs.get(params.runId);
        if (run) {
          run.end(params);
        } else {
          this.finishedEarly.set(params.runId, params);
        }
        this.runs.delete(params.runId);
        break;
      }
      case "stream.lagged": {
        this.options.onStreamLagged?.(params);
        break;
      }
      case "approval.requested": {
        this.enqueueCallback(() => this.handleApproval(params));
        break;
      }
      case "input.requested": {
        this.enqueueCallback(() => this.handleQuestion(params));
        break;
      }
      case "host_tool.invoke": {
        this.enqueueCallback(() => this.handleHostTool(message));
        break;
      }
      default:
        break;
    }
  }

  enqueueCallback(callback) {
    this.callbackQueue.push(callback);
    if (this.callbackRunning) return;
    this.callbackRunning = true;
    setImmediate(async () => {
      while (this.callbackQueue.length) {
        const next = this.callbackQueue.shift();
        try {
          await next();
        } catch (error) {
          this.options.onError?.(error);
        }
      }
      this.callbackRunning = false;
    });
  }

  async handleApproval(params) {
    let allow = false;
    if (typeof this.options.onApproval === "function") {
      const decision = await this.options.onApproval(params);
      allow = decision === true || decision === "allow";
    }
    await this.request("approval.resolve", {
      sessionId: params.sessionId,
      approvalId: params.approvalId,
      decision: allow ? "allow" : "deny",
    }).catch(() => {});
  }

  async handleQuestion(params) {
    let answer = null;
    if (typeof this.options.onQuestion === "function") {
      answer = await this.options.onQuestion(params);
    }
    await this.request("input.answer", {
      sessionId: params.sessionId,
      requestId: params.requestId,
      answer: answer ?? null,
    }).catch(() => {});
  }

  async handleHostTool(message) {
    const { callId, tool } = message.params;
    const handler = this.tools.get(tool)?.handler;
    if (!handler) {
      this.send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32010,
          message: `host tool ${tool} has no handler`,
          data: { kind: "host_tool_failed" },
        },
      });
      return;
    }
    try {
      const result = await handler(message.params.arguments ?? {});
      this.send({
        jsonrpc: "2.0",
        id: message.id,
        result: normalizeToolResult(result),
      });
    } catch (error) {
      this.send({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32010,
          message: error?.message ?? String(error),
          data: { kind: "host_tool_failed" },
        },
      });
    }
  }

  adoptRun(run) {
    this.runs.set(run.id, run);
    const buffered = this.buffered.get(run.id) ?? [];
    for (const event of buffered) run.push(event);
    this.buffered.delete(run.id);
    const finished = this.finishedEarly.get(run.id);
    if (finished) {
      this.finishedEarly.delete(run.id);
      run.end(finished);
    }
  }

  async createSession(spec = {}) {
    const result = await this.request("session.create", spec);
    return new Session(this, result.sessionId);
  }

  async openSession(sessionId) {
    await this.request("session.project", { sessionId });
    return new Session(this, sessionId);
  }

  async listSessions() {
    const result = await this.request("session.list");
    return result.sessions;
  }

  async archiveSession(sessionId) {
    return this.request("session.archive", { sessionId });
  }

  async *subscribe(sessionId, afterSequence = 0) {
    const result = await this.request("session.subscribe", { sessionId, afterSequence });
    const subscriptionId = result.subscriptionId;
    const queue = [];
    const waiters = [];
    const subscription = {
      sessionId,
      queue,
      push(value) {
        const waiter = waiters.shift();
        if (waiter) waiter(value);
        else queue.push(value);
      },
      end() { for (const waiter of waiters.splice(0)) waiter(END); },
      next() {
        if (queue.length) return Promise.resolve(queue.shift());
        return new Promise((resolve) => waiters.push(resolve));
      },
    };
    this.subscriptions.set(subscriptionId, subscription);
    try {
      while (true) {
        const event = await subscription.next();
        if (event === END) return;
        yield event;
      }
    } finally {
      this.subscriptions.delete(subscriptionId);
      await this.request("session.unsubscribe", { subscriptionId }).catch(() => {});
    }
  }

  async close() {
    if (this.closed) return;
    await this.request("agent.shutdown", { cancelRunning: true, timeoutMs: 5000 }).catch(() => {});
    this.child.stdin.end();
    await new Promise((resolve) => {
      if (this.child.exitCode !== null) return resolve();
      this.child.once("exit", resolve);
      setTimeout(() => this.child.kill(), 6000).unref?.();
    });
    this.closed = true;
  }
}
