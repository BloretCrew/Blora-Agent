# GitHub

协作者在 Issue 或 Pull Request 评论里写 `/blora`、`/ba` 或 `@blora`，Actions 会在该仓库的 runner 上运行 Blora Agent。触发词按词边界匹配，大小写不敏感，所以 `foo/blora` 和 `a@blora.com` 不会触发。

评论内容决定结果。解释、总结和审查只回复评论。对方明确要求修改时，代理改当前分支；需要推送或打开 Pull Request 时，由代理自己调用 `git` 和 `gh`。工作流不按事件类型自动开 PR。代理留下的未提交改动，会在当前分支不是默认分支时被提交并推送。

## 接入

```bash
blora github install
```

命令把 `.github/workflows/blora.yml` 写到当前工作区。文件已存在时需要 `--force`。`--provider` 和 `--model` 会写进工作流；省略时沿用 runner 上的 `BLORA_PROVIDER` 和 `BLORA_MODEL`。

仓库 Actions secrets 至少要有 `BLORA_API_KEY`。推送和评论使用 `GITHUB_TOKEN`。需要访问 fork 或更高权限时，另加 secret `BLORA_GITHUB_TOKEN`。

工作流权限是 `contents: write`、`issues: write`、`pull-requests: write`。checkout 使用 `persist-credentials: false`。Action 在启动前准备 `GH_TOKEN` 和 `GIT_ASKPASS`，代理自己执行的 `git push` 与 `gh` 用这套凭证。编排器代为推送时，token 只出现在当次 git 命令的 `http.extraheader` 里，不写入 `.git/config`。

触发者必须通过协作者权限接口被判定为 `write` 或 `admin`。`schedule` 没有触发者，跳过这步。

## 事件

`install` 只订阅 `issue_comment` 和 `pull_request_review_comment`。runner 另外接受 `issues`、`pull_request`、`schedule` 和 `workflow_dispatch`。后三者里，`issues`、`schedule`、`workflow_dispatch` 必须由 action 输入 `prompt` 提供任务；单独的 `pull_request` 事件在没有 prompt 时只做审查，不改文件。

Issue 会先切到 `blora/issue{编号}-{时间}`，避免直接改默认分支。同仓库 PR 检出 head 分支。fork PR 检出到本地 `blora/pr{编号}-{时间}`，推送目标是 fork。

行内 review 评论会把文件、行号和 diff 放进提示词。评论只有触发词时，默认是总结或审查，不改文件。

## 试跑

```bash
GITHUB_RUN_ID=dummy \
  blora github run --event ./event.json --dry-run
```

事件文件可以是 webhook 原文，也可以是带 `eventName`、`repo`、`actor`、`payload` 的包装 JSON。`--dry-run` 不调用模型，也不推送。

## 限制

没有托管 GitHub App，也没有公开的会话分享页。fork 上的推送失败会写进评论，作业失败。图片附件保存为本地文件，`read_file` 只返回类型和尺寸。`yolo` 仍会拒绝危险命令。提交时排除 `.blora/`、`.env`、`.env.local` 和 `.env.*.local`。

Action 定义在仓库的 `github/action.yml`。发布物还没准备好时，先把 `blora` 放进 PATH。
