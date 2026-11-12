# 媒体能力

媒体工具通过 OpenAI 兼容的 HTTP 端点工作，配置来自环境变量：

| 变量 | 默认 | 用途 |
|---|---|---|
| `BLORA_MEDIA_KEY` / `BLORA_API_KEY` / `OPENAI_API_KEY` | — | 认证 |
| `BLORA_MEDIA_BASE` / `BLORA_IMAGE_BASE` / `BLORA_API_BASE` | `https://api.openai.com/v1` | 端点 |
| `BLORA_IMAGE_MODEL` | `gpt-image-1` | 图片编辑 |
| `BLORA_VISION_MODEL` | `gpt-4o` | 图像分析 |
| `BLORA_VIDEO_MODEL` | `sora-2` | 视频生成 |
| `BLORA_TTS_MODEL` | `tts-1` | 语音合成 |
| `BLORA_TTS_VOICE` | `alloy` | 默认音色 |

## 工具

- `edit_image`：`/images/edits` multipart，输入 `path` 或 base64 `image`，可带 `mask_path`，返回编辑后的图片。
- `generate_image`：带 `image_path` 时走图生图（`/images/edits`），否则走文生图。
- `generate_video`：`/videos/generations`，支持 `image_path`、`duration_seconds`、`aspect_ratio`；同步返回文件或 `id` 任务号。
- `text_to_speech`：`/audio/speech`，返回 mp3/wav/opus/aac/flac 文件。
- `vision_analyze`：`/chat/completions`，把图片作为 data URL 发送，返回文本分析。

图片返回图像内容块；音频/视频返回 `File` 附件引用。所有文件写入 `.blora/attachments/`。

## 策略

`Policy::media_generate` 控制：Plan 拒绝，Ask 审批，AutoEdit/Yolo 允许。网络仍受 `BLORA_NETWORK` 约束（实际 HTTP 调用由 ureq 直接发出，策略在工具入口检查）。

## 模式

- `edit_image`、`generate_video`、`text_to_speech`：Imagine、Work、Agent。
- `vision_analyze`：Code、Work、Agent，供非视觉主模型或多模型复核使用。
