# 待办

## 当前任务：方案 A

- [ ] 修复 Windows `bash`/`sh` 探测：不用 `--version`，改用无害 POSIX 语法探针校验
- [ ] 生成并接入稳定的 Windows 宿主 shell 行为测试
- [ ] 重新运行失败的 `shell_cwd_persists_across_calls`
- [ ] 重新运行 `blora-exec`、`blora-runtime` 相关测试
- [ ] 视测试结果提交修复

## 环境备注

- 本机已配置 MinGW（gcc 16.2.0），但 Windows 宿主 shell 探测仍需修正。
