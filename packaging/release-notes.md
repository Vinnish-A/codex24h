Linux x86_64 / WSL 预编译版本（glibc 2.35+，例如 Ubuntu 22.04+）。

- 修复开启邮件后 `resume` 与共享后台会话竞争：共享模式使用当前 TUI 的临时事件管道及只读完成元数据，不再加入会触发独立写入者的 `notify` 配置覆盖。
- 保留原生 `← for agents`、Esc 返回和 Tab 补全；独立模式和显式 `--no-daemon` 保留原生通知路径。
- 同时验证 Codex CLI **0.158.0、0.159.2、0.160.0**，共享后台为 **0.160.0 app-server**。每次启动使用当前 PATH 或 `CODEX24H_CODEX` 选定的 Codex；CLI 与后台版本可不同，具体范围见 README。
- 安装下载有超时，支持本地安装包；预编译包及 SHA-256 校验文件随 Release 提供，更新不影响运行中的旧进程。

共享模式完成邮件在 wrapper 在线期间观察。完整 UUID 恢复可观察已在运行的轮次；通过 picker、`--last` 或 agents 仅接入旧任务时，提交过本次 TUI 的请求后才能建立通知归属。脱离 TUI 后的后台完成和旧后台服务组合不在本轮保证范围，详见 `docs/usage.md` 和 `TESTING.md`。

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

`codex` 仍需已安装并登录；接管 tmux 会话时需要 tmux 和 tic。
