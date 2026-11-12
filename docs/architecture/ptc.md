# Programmatic tool calling (`execute_code`)

`execute_code` 让模型写一小段 Python 或 JavaScript，在脚本里直接调用 Blora 工具，减少多轮 tool-call 往返。

## 工具参数

```json
{
  "language": "python",
  "code": "text = call_tool('read_file', {'path': 'README.md'})\nfinish(text)",
  "timeout_seconds": 60,
  "max_tool_calls": 50
}
```

- `language`：`python` 或 `javascript`。
- `timeout_seconds`：1–600，默认 60。
- `max_tool_calls`：1–200，默认 50。

## 脚本 API

Python：

```python
text = call_tool("read_file", {"path": "src/lib.rs"})
log("read %d bytes" % len(text))
finish({"length": len(text)})
```

JavaScript：

```js
const text = await callTool("read_file", {path: "src/lib.rs"});
log(`read ${text.length} bytes`);
finish({length: text.length});
```

- `call_tool(name, arguments)` / `callTool(name, args)`：同步（Python）或 Promise（JS）调用允许的 Blora 工具。
- `log(text)`：写入工具输出。
- `finish(value)`：结束脚本并返回结构化结果。

## 工具白名单

允许：文件读写、`list_dir`、`search`、`glob`、`grep`、`view_image`、`apply_patch`、`shell`、Git、`web_*`、`read_document`、`notebook_edit`、`process`。

拒绝：`execute_code` 自身、`subagent`/`delegate`、`ask_user`、`exit_plan_mode`、`artifact`、`schedule`、`team`/`team_task`/`goal`、`remember`/`forget`、`skill_manage`、`handoff`，以及所有 `mcp__*`/`plugin__*`。

## 桥协议

- 父进程监听 `127.0.0.1:0`，把端口和随机 token 通过环境变量传给子进程。
- 子进程先发送 `{"method":"hello","token":"..."}`，父进程校验后回复 `{"ok":true}`。
- 每次调用：`{"id":1,"method":"call","params":{"tool":"read_file","arguments":{...}}}`，父进程回复 `{"id":1,"ok":true,"result":"..."}`。
- `finish` 和 `log` 是单向消息。
- 脚本超时、工具调用超限或父进程关闭连接都会终止子进程。

## 安全

- `Policy::code_exec` 控制启动：Plan 拒绝，Ask 询问，AutoEdit/Yolo 允许。
- 脚本本身不受 Blora 的 shell 分段策略约束；它是受审批保护的通用执行入口。
- 子进程 cwd 是工作区；允许的工具仍各自经过 Blora 的路径、网络和密钥策略。
- 不要在没有审批的情况下让不可信模型写脚本访问网络或删除文件。
