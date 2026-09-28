# 邮件通知

启用后，每轮回答结束会自动发送一封邮件，附上会话名、Session ID、完成时间和恢复命令。若这一轮完成了 Goal，标题和正文会标注 **Goal 已完成**。发信由本机程序执行，不需要 agent 调用工具或生成邮件。

邮件标题示例：

```text
[Codex24h][本轮已完成] 整理实验数据
[Codex24h][Goal 已完成] 整理实验数据
```

## 配置

需要 Python 3.11 或更高版本，无需额外 Python 包。重新运行 `./install.sh` 会安装 `codex24h-mail`。

```bash
mkdir -p ~/.config/codex24h
cp config/mail.example.toml ~/.config/codex24h/mail.toml
chmod 600 ~/.config/codex24h/mail.toml
```

编辑 `mail.toml`，填写 SMTP 服务器、发件邮箱和收件邮箱。例如网易 163 邮箱：

```toml
enabled = false
host = "smtp.163.com"
port = 465
security = "ssl"
from = "your-name@163.com"
to = ["recipient@example.com"]
username = "your-name@163.com"
password_file = "smtp-password"
```

`ssl` 使用隐式 TLS，常见端口为 465；`starttls` 使用 STARTTLS，常见端口为 587。按邮件服务商提供的参数填写。连接会验证服务器证书，不支持明文认证。

私有 SMTP 服务如使用自己的证书颁发机构，可用 `ca_file` 指定受信任的 CA 文件；省略时使用系统信任库。

开启邮箱的 SMTP 服务后，在本机隐藏输入客户端授权码，然后发送测试邮件：

```bash
codex24h-mail set-password
codex24h-mail test
```

授权码写入本机文件，权限为 0600，不进入项目或命令参数。网易等邮箱通常使用专门的客户端授权码，而非网页登录密码。也可改用 `password_env` 指定环境变量，但该变量必须在执行通知的 Codex 进程环境中可用；共享后台进程下推荐使用文件。

测试成功后将 `enabled` 改为 `true`，重新启动会话即可：

```bash
codex24h
codex24h resume --last
```

已有的运行中会话需要重新启动或恢复，才能接入通知。也支持 `codex24h exec` 的完成通知。

## 失败与重试

发信在独立后台进程中完成，SMTP 延迟不会阻塞 TUI。临时网络错误最多自动重试一次；认证错误不会自动重试。失败记录保存在本机队列，修好配置后可重试：

```bash
codex24h-mail status
codex24h-mail retry
```

同一会话的同一轮通过 Session ID 和 Turn ID 去重。SMTP 接收成功表示已交给邮件服务器，不保证最终进入收件箱；网络中断恰好发生在服务器接收之后时，重试仍可能导致重复邮件。

临时关闭通知：

```bash
CODEX24H_MAIL=0 codex24h resume --last
```

长期关闭可将配置中的 `enabled` 改为 `false`。使用 `CODEX24H_MAIL_CONFIG` 指定其他配置文件；辅助命令也支持 `--config PATH`。队列默认位于 `~/.local/state/codex24h/mail/`，尊重 `XDG_STATE_HOME`，也可用配置项 `state_dir` 修改。

## 触发方式与范围

使用 Codex 原生的 [`notify` / `agent-turn-complete` 事件](https://learn.chatgpt.com/docs/config-file/config-advanced#notifications)。Wrapper 仅为本次启动附加通知配置，不修改 Codex 全局配置；原有通知命令仍会执行。没有轮询终端文字，没有用模型判断是否发信。

会话名称和 Goal 状态从本机 Codex 元数据只读获取。目前适配本机 Codex 0.158.0 的状态库：名称优先使用手动命名，其次使用 Codex 保存的标题；取不到时以 Session ID 标识。邮件不附带对话正文。子 agent 的独立完成事件不发送邮件。

Goal 必须在本次接入通知之后变为 `complete` 才会标注，且每个 Goal 只标注一次；恢复一个早已完成的 Goal 不会误报为新完成。它代表 Codex 的 Goal 状态，不额外判断任务的实际质量。

本功能针对本机 Codex；远程 app-server 不在支持范围内。Codex 内部状态库的未来变更可能使名称回退到 ID 或无法标注 Goal，普通完成通知仍以原生事件为准。

## 验证

```bash
cargo build --locked
python3 tests/mail.py

# 可选：发送两次真实模型请求，验证 exec 和 resume TUI 自动通知。
# 邮件只投递给本机测试 TLS SMTP 服务，不发到外部邮箱。
python3 tests/native_mail.py
```
