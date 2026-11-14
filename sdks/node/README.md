# @bloret-crew/blora-agent-sdk

Node.js/TypeScript client for the Blora Agent stdio protocol.

## Install

```bash
npm install @bloret-crew/blora-agent-sdk
```

The SDK spawns the `blora` binary. Resolve it with `binaryPath`, `BLORA_BIN`,
or the `blora` command on `PATH` (the `@bloret-crew/blora-agent` installer
provides it).

## Web/chat agent

```js
import { Agent } from "@bloret-crew/blora-agent-sdk";

const agent = await Agent.start({
  profile: "web",
  provider: {
    kind: "openai-compatible",
    baseUrl: "https://api.example.com/v1",
    apiKey: process.env.MODEL_KEY,
    model: "chat-1",
  },
  tools: [{
    name: "search_docs",
    description: "Search product documentation",
    parameters: {
      type: "object",
      properties: { q: { type: "string" } },
      required: ["q"],
    },
    readOnly: true,
    handler: async ({ q }) => `Results for ${q}`,
  }],
  onApproval: async () => "deny",
  onQuestion: async () => null,
});

const session = await agent.createSession();
const run = await session.run("How do I reset my password?");
for await (const event of run.events()) {
  if (event.type === "assistant.delta") {
    process.stdout.write(event.payload.text ?? "");
  }
}
console.log(await run.wait());
await agent.close();
```

## License

GPL-3.0-or-later. See the repository `LICENSE`.
