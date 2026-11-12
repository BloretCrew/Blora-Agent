# LSP

Blora 通过 `lsp` 工具查询语言服务器，提供诊断、悬停、跳转、引用和符号列表。

## 工具

`lsp { action, path, line?, character?, query?, timeout_seconds? }`

- `diagnostics`：打开文档并等待 `textDocument/publishDiagnostics`。
- `hover`：返回 Markdown/文本形式的类型与文档。
- `definition`：返回 `file://...:line:column`。
- `references`：返回引用位置列表。
- `symbols`：当前文档的符号树。
- `workspace_symbols`：按 `query` 搜索工作区符号。

`line` / `character` 是 1-based，内部转换为 LSP 的 0-based。

## 配置

按顺序读取 `<workspace>/.blora/lsp.json` 和 `$BLORA_HOME/lsp.json`，未配置时使用内置默认值：

```json
{
  "servers": {
    "rust": {
      "command": "rust-analyzer",
      "args": [],
      "root_markers": ["Cargo.toml"],
      "language_ids": {"rs": "rust"},
      "env": {},
      "timeout_secs": 10
    }
  }
}
```

内置服务器：

| 语言 | 命令 | 根标记 |
|---|---|---|
| Rust | `rust-analyzer` | `Cargo.toml` |
| TypeScript/JavaScript | `typescript-language-server --stdio` | `package.json`、`tsconfig.json`、`jsconfig.json` |
| Python | `pyright-langserver --stdio` | `pyproject.toml`、`setup.py`、`requirements.txt` |
| Go | `gopls` | `go.mod` |
| C/C++ | `clangd` | `compile_commands.json`、`CMakeLists.txt` |

扩展名映射由 `language_ids` 决定；自定义服务器必须提供该映射。

## 生命周期

- 客户端按 `(server, root)` 缓存；首次调用时惰性启动。
- 文档首次访问时 `didOpen`，之后复用同一连接。
- 每个请求有超时（默认 10 秒，最大 120 秒）。
- 进程退出后下一次调用会重新连接。
- 服务器进程通过 `Policy::lsp_server` 约束：Plan 模式允许只读查询，Ask 模式首次启动需要审批。

## 验证

```sh
cargo test -p blora-lsp
```

仓库内 `scripts/fake_lsp_server.py` 实现 initialize、didOpen、diagnostics、hover、definition、references、documentSymbol、workspace/symbol，可用于本地验收。
