# Development

```bash
scripts/check.sh
cargo test --workspace
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli -- tui
cargo run -p blora-cli -- serve --bind 127.0.0.1:8787
```

`BLORA_HOME` 可指向临时目录做隔离测试。真实模型使用 `BLORA_API_KEY` / `BLORA_API_BASE` / `BLORA_MODEL`。
