// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

import test from "node:test";
import assert from "node:assert/strict";
import { Agent } from "../index.js";

const binaryPath = process.env.BLORA_BIN;

test("basic scripted chat", async () => {
  const agent = await Agent.start({
    binaryPath,
    profile: "web",
    provider: {
      kind: "scripted",
      completions: [{ text: "hello from node", finish_reason: "stop" }],
    },
  });
  const session = await agent.createSession();
  const run = await session.run("hi");
  let text = "";
  for await (const event of run.events()) {
    if (event.type === "assistant.delta") text += event.payload.text ?? "";
  }
  const outcome = await run.wait();
  assert.equal(outcome.status, "completed");
  assert.equal(outcome.finalText, "hello from node");
  assert.equal(text, "hello from node");
  await agent.close();
});

test("host tool round-trip", async () => {
  const agent = await Agent.start({
    binaryPath,
    profile: "web",
    provider: {
      kind: "scripted",
      completions: [
        {
          tool_calls: [
            { id: "call_echo", name: "echo", arguments: '{"text":"hi"}' },
          ],
          finish_reason: "tool_calls",
        },
        { text: "done", finish_reason: "stop" },
      ],
    },
    tools: [
      {
        name: "echo",
        description: "Echo text",
        parameters: { type: "object", properties: { text: { type: "string" } } },
        readOnly: true,
        handler: async ({ text }) => {
          const session = await agent.createSession();
          await session.project();
          return `echo:${text}`;
        },
      },
    ],
  });
  const session = await agent.createSession();
  const run = await session.run("use echo");
  const outcome = await run.wait();
  assert.equal(outcome.status, "completed");
  assert.equal(outcome.finalText, "done");
  await agent.close();
});
