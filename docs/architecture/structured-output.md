# 结构化输出

Blora 支持两种结构化输出路径。

## 原生 response_format

客户端在 `run` 请求中传入 `output_schema`：

```json
POST /api/sessions/:id/run
{"prompt": "...", "output_schema": {"type": "object", "properties": {"answer": {"type": "string"}}, "required": ["answer"]}}
```

运行时把它映射为：

- OpenAI Chat：`response_format.json_schema`
- OpenAI Responses：`text.format.json_schema`
- Gemini：`generationConfig.responseMimeType/responseSchema`
- Anthropic：强制 `structured_output` 工具（`tool_choice`）

## `structured_output` 工具

模型也可以显式调用：

```json
{"schema": {...}, "data": {...}}
```

运行时校验 `required`、`properties`、`items` 和基本类型，然后写入 `structured.output` 事件。`SessionJson.structured_output` 保存最新值，供 Web/TUI 按 schema 渲染。

## 兼容性

- 旧会话没有 `structured_output` 字段时按 `null` 读取。
- 校验失败返回可读错误，模型可以修正后重试。
- 结构化输出不改变普通工具循环；模型仍可在同一次运行中继续调用工具。
