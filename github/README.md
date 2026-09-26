# 在 GitHub 里调用 Blora Agent

仓库协作者在 Issue 或 Pull Request 里写下 `/blora`、`/ba` 或 `@blora` 后，GitHub Actions 会在这个仓库的 runner 上启动 Blora Agent。

评论里的原话决定它做什么。只要求解释或审查时，它回复评论，不改文件。明确要求修复或修改时，它改代码；需要推送或打开 Pull Request 时，由它自己用 `git` 和 `gh` 完成。工作流不会因为事件类型自动开 PR。没提交的改动会在当前分支不是默认分支时被提交并推送，避免 runner 结束时丢掉。

## 安装

在目标仓库里运行：

```bash
blora github install
```

然后：

1. 提交并推送 `.github/workflows/blora.yml`
2. 在仓库的 Actions secrets 里添加 `BLORA_API_KEY`
3. 用有写权限的账号评论，例如 `/blora 总结这个问题` 或 `@blora 修好登录失败`

也可以指定供应商：

```bash
blora github install --provider openai --model gpt-4o-mini
```

## 评论示例

解释一个 Issue，不改代码：

```text
/blora 解释一下这个问题
```

按你的要求修：

```text
@blora 修好这个问题，并打开 Pull Request
```

在 PR 的 Files 页对某几行评论：

```text
/ba 给这里补上错误处理
```

触发者必须是仓库的 `write` 或 `admin`。

## 手动工作流

Action 位于 `bloret-crew/blora-agent/github`：

```yaml
name: blora

on:
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]

jobs:
  blora:
    if: |
      contains(github.event.comment.body, '/blora') ||
      contains(github.event.comment.body, '/ba') ||
      contains(github.event.comment.body, '@blora')
    runs-on: ubuntu-latest
    permissions:
      contents: write
      issues: write
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 1
          persist-credentials: false
      - uses: bloret-crew/blora-agent/github@main
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          BLORA_API_KEY: ${{ secrets.BLORA_API_KEY }}
          BLORA_NETWORK: "1"
```

`schedule`、`workflow_dispatch` 和 `issues` 没有评论正文，要在 action 的 `prompt` 输入里写任务。`blora github install` 默认不订阅这三类事件。

## 本地试跑

```bash
GITHUB_RUN_ID=dummy GITHUB_TOKEN=... \
  blora github run --event ./event.json --dry-run
```

`--dry-run` 只打印将要送给模型的提示词。

## 限制

- 认证用 `GITHUB_TOKEN`，或 secret `BLORA_GITHUB_TOKEN`（PAT 或 GitHub App token）。没有托管的令牌交换服务。
- `GITHUB_TOKEN` 通常推不回 fork。需要改 fork 上的 PR 时，换成对那个 fork 有写权限的 token。
- 不会把改动推送到仓库的默认分支。
- 图片附件会保存成文件。当前模型输入看不到像素，`read_file` 只描述类型和尺寸。
- 危险命令（例如 `git push --force`、`rm -rf`）在 `yolo` 下仍会失败，CI 里不会等待人工批准。
- 在发布对应平台的 release 资产之前，action 需要 PATH 里已经有 `blora`。
