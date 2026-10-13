Linux x86_64 / WSL 容量检测与自动恢复（glibc 2.35+）。

- 修复共享后台下界面显示 capacity 却无邮件的问题：按当前前端确认会话的结构化失败轮次识别，不再要求前端持有日志。
- 每次容量错误独立随机等待 3～10 秒。普通任务自动提交 `go on`；仅 paused Goal 提交 `/goal resume`，complete / achieved 不恢复。新建 Goal 通过本前端原生 `/status` 确认 Session ID，再读取状态。
- Codex 确认接收后发送“自动恢复请求已提交”邮件。每会话连续最多三次，第四次仅发“容量不足，请检查”；成功或手动新请求重置，自动回复与重复报错不重置。
- 有草稿、输入、会话切换或状态不明时不盲发；恢复中断不重放指令。邮件重试仅重发邮件。保留主会话归属过滤、隐私与 SMTP TLS 校验。
- `/resume` 同步监控目标。共享及独立模式均用真实 Codex 配合本地模拟容量错误和 TLS SMTP 验证；安装更新继续保留已有邮件配置、授权码和队列。

更新：

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

更新后重新打开 `codex24h` 或 `codex24h resume SESSION_UUID` 加载新功能，无需重启共享后台。`exec` 和远程 app-server 尚不支持容量自动恢复。
