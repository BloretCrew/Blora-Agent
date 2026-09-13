# Development

```bash
scripts/check.sh
cargo test --workspace
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli --
cargo run -p blora-cli -- web --bind 127.0.0.1:8787
cargo run -p blora-cli -- session fork ses_…
cargo run -p blora-cli -- session export ses_…
```

`BLORA_PROVIDER` 可写 `openai,anthropic`。`BLORA_WORKTREE=1` 在 git worktree 中执行。`BLORA_HOOKS_DIR` 可放 `session-start` / `tool-before` / `tool-after` 可执行文件。`BLORA_MCP_COMMAND` 启动 stdio MCP。

`BLORA_HOME` 可指向临时目录做隔离测试。真实模型使用 `BLORA_API_KEY` / `BLORA_API_BASE` / `BLORA_MODEL`。
