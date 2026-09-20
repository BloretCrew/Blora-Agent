# Blora Agent 翻译源文件

本目录中的 JSON 文件是上传到 Bloret Translation Collector 的源文件模板。

## 上传

将 `zh-CN.json` 作为 Blora Agent 项目的源文件上传，上传成功后服务端 Manifest 会返回 `defaultFileId`。

实时译文接口：

```text
GET https://tr.bloret.net/api/v1/orgs/bloret/projects/blora-agent/files/{fileId}/translated?locale=en&mode=top_voted&fallbackMt=1
```

## 本地测试

```bash
BLORA_LOCALE=en \
BLORA_TRANSLATION_FILE_ID=<file-id> \
 cargo run -p blora-cli
```

如果未设置 `BLORA_TRANSLATION_FILE_ID`，客户端会尝试从项目 Manifest 自动发现 `defaultFileId` 或第一个文件。

## 配置项

- `BLORA_LOCALE`：语言代码，默认 `zh-CN`。
- `BLORA_TRANSLATION_BASE_URL`：翻译服务地址，默认 `https://tr.bloret.net`。
- `BLORA_TRANSLATION_ORG`：组织 slug，默认 `bloret`。
- `BLORA_TRANSLATION_PROJECT`：项目 slug，默认 `blora-agent`。
- `BLORA_TRANSLATION_FILE_ID`：可选，源文件 ID；不设置时从 Manifest 自动发现。

网络不可用、项目没有源文件或译文请求失败时，客户端继续使用内置 `zh-CN` 文案。
