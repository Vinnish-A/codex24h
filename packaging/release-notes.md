Linux x86_64 / WSL 预编译版本（glibc 2.35+，例如 Ubuntu 22.04+）。

- 移除旧终端镜像接管。tmux **Ctrl+B，然后 h** 选择共享主会话后打开正常 codex24h 前端，支持滚动、实时输入区和搜索；退出新 Codex 或 **Ctrl+]，然后 d** 关闭弹窗并返回原页面。
- 原页面、草稿和后台任务保留；新前端不复制原草稿。跟随原页面正在使用的 Codex 版本，保留原生 `← for agents`。
- 重连请求列表按明确选择的 UUID 读取共享历史，范围固定为所选主会话，不自动跟随原生 agents、`/new` 或 `/resume` 切换。
- 子 agent 与身份不明的会话禁止自动发邮件；实际发送前复查队列，旧子任务通知也不再投递。

快捷键配置和适用范围见 [重连说明](https://github.com/Vinnish-A/codex24h/blob/main/docs/attach.md)。旧 `attach <PID>` 和 `--list` 入口移除，不再需要 `tic`。普通启动继续支持 README 所列 Codex 版本。

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

已安装并登录 Codex 即可使用；预编译包不要求用户安装 Rust、Python 或 pip。tmux 快捷键另外需要支持 `display-popup` 的 tmux。
