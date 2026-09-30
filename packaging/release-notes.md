Linux x86_64 / WSL 预编译版本（glibc 2.35+，例如 Ubuntu 22.04+）。

- 一行安装和更新，用户无需 Rust、Python 或 pip；四个辅助功能共用随包运行时。
- 邮件通知不再依赖服务器的 tomllib，支持完成、Goal 完成及模型容量不足分类。
- 下载包附 SHA-256 校验，安装后清理临时文件；运行中的会话继续使用原版本。
- 邮件配置见仓库 docs/mail.md，授权码需在服务器私下设置。

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

`codex` 仍需已安装并登录；接管 tmux 会话时需要 tmux 和 tic。
