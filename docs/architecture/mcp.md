# MCP

Blora 支持多个 MCP 服务器，工具、resources 和 prompts 都可从模型侧访问。

## 配置

按优先级读取：

1. `BLORA_MCP_CONFIG` 指向的文件
2. `<workspace>/.blora/mcp.json`
3. `$BLORA_HOME/mcp.json`（默认 `~/.blora/mcp.json`）
4. 兼容旧配置 `BLORA_MCP_COMMAND`

```json
{
  "servers": {
    "github": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {"GITHUB_TOKEN": "${GITHUB_TOKEN}"},
      "cwd": ".",
      "enabled": true,
      "trusted": false,
      "read_only": false,
      "timeout_secs": 30
    },
    "remote": {
      "transport": "http",
      "url": "https://example.com/mcp",
      "headers": {"Authorization": "Bearer ${MCP_TOKEN}"}
    }
  }
}
```

`${ENV}` 会从进程环境展开；token 不写入事件或日志。`enabled=false` 跳过该服务器。未填 `transport` 时默认 stdio。

## 工具命名

- 多服务器：`mcp__{server}__{tool}`
- 只有一个兼容服务器 `default`（来自 `BLORA_MCP_COMMAND`）：保留旧的 `mcp__{tool}`

MCP 工具默认**不**直接进入声明。模型调用 `tool_search`（可按 `source="mcp"` 过滤），命中的工具写入 `session_tools`，下一轮请求才带上完整 schema。设置 `BLORA_TOOL_SEARCH=0` 可恢复一次性声明。

## Resources 与 Prompts

- `mcp_list_resources` / `mcp_read_resource`：列出和读取服务器资源；文本内容直接返回，二进制 blob 返回大小说明。
- `mcp_list_prompts` / `mcp_get_prompt`：列出和渲染 prompt 消息。

这四个工具是只读的，不需要网络权限；服务器启动本身受 `Policy::mcp_server` 约束（plan 模式允许只读查询，ask 模式首次连接需要审批）。

## 传输

- **stdio**：行分隔 JSON-RPC；stderr 被忽略；服务器崩溃后下一次调用会重新连接。
- **HTTP**：POST JSON-RPC，支持 JSON 或 `text/event-stream` 响应；`Mcp-Session-Id` 会保留并在后续请求中回传。

## 手动验证

```sh
BLORA_MCP_CONFIG=/tmp/mcp.json cargo run -p blora-cli -- run --mock "list mcp tools"
```

仓库内 `scripts/fake_mcp_server.py` 提供 tools/resources/prompts 的最小实现：

```json
{"servers":{"fake":{"command":"python3","args":["scripts/fake_mcp_server.py"]}}}
```
