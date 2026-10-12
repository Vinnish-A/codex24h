Linux x86_64 / WSL 预编译版本（glibc 2.35+，例如 Ubuntu 22.04+）。

- 一键脚本区分首次安装、版本更新、同版本刷新命令入口和旧源码安装升级；版本更新显示旧、新版本。保留邮件配置、SMTP 授权码和发信队列，无需重新配置邮箱。
- 修复普通 `codex24h resume SESSION_UUID` 在共享模式下请求列表为空的问题，按目标 UUID 读取共享历史，必要时读取该会话日志。
- 修复空会话内 `/resume` 以及再次切换会话后的请求检索。列表跟随原生选择器确认的会话，并清除旧缓存；窄窗口中的截断 UUID 仅在唯一匹配时使用。原生 agents 和 `/new` 切换仍不在自动跟踪范围内。
- 增加 Codex CLI **0.162.0** 实机验证：恢复会话、原生 agents、Tab 补全、完成邮件和请求检索。保留原先已验证的 0.158.0、0.159.2、0.160.0。
- 适配全文搜索的 `enter accept` 页脚，解决定位完成后仍显示 searching、滚动等待不释放的问题，继续支持旧版页脚。

0.162.0 验证复用本机 0.162.1 app-server，以 `--disable api_key_model_discovery` 对齐已有后台的 false 设置，未重启后台。包装器不自行更改后台共享 feature；设置不一致时仍由原生 Codex 提示处理。验证组合与范围见 [测试记录](https://github.com/Vinnish-A/codex24h/blob/main/TESTING.md)。

安装或更新：

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

脚本下载最新已发布的 Release。预编译包不要求用户安装 Rust、Python 或 pip，继续使用本机选定的 Codex 和已有登录。
