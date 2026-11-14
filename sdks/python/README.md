# blora-agent-sdk

Python client for the Blora Agent stdio protocol.

```bash
pip install blora-agent-sdk
```

The SDK spawns the `blora` binary. Set `BLORA_BIN` or pass `binary_path`; the
`blora` command on `PATH` is also used.

```python
from blora_agent import Agent

agent = Agent.start(
    profile="web",
    provider={
        "kind": "openai-compatible",
        "baseUrl": "https://api.example.com/v1",
        "apiKey": "…",
        "model": "chat-1",
    },
    tools=[{
        "name": "search_docs",
        "description": "Search product docs",
        "parameters": {"type": "object", "properties": {"q": {"type": "string"}}},
        "read_only": True,
        "handler": lambda args: f"Results for {args['q']}",
    }],
    on_approval=lambda request: "deny",
)

session = agent.create_session()
run = session.run("How do I reset my password?")
for event in run.events():
    if event["type"] == "assistant.delta":
        print(event["payload"].get("text", ""), end="")
print(run.wait())
agent.close()
```

## License

GPL-3.0-or-later. See the repository `LICENSE`.
