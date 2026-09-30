# 邮件通知

启用后，每轮回答结束会自动发送一封邮件，附上会话名、Session ID、完成时间和恢复命令。若这一轮完成了 Goal，标题和正文会标注 **Goal 已完成**。发信由本机程序执行，不需要 agent 调用工具或生成邮件。

邮件标题示例：

```text
[Codex24h][本轮已完成] 整理实验数据
[Codex24h][Goal 已完成] 整理实验数据
[Codex24h][模型容量不足] 整理实验数据
```

## 新服务器配置

以下操作在 **运行 codex24h 的 Linux / WSL 服务器上**完成，使用运行 Codex 的同一个用户。无需在 SSH 客户端电脑上配置邮件，也不需要克隆仓库。

### 1. 安装并准备邮箱

预编译 Release 已包含邮件程序及运行时，服务器不需要安装或升级 Python。已安装新版 codex24h 可跳过安装命令：

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
codex24h-mail --help
```

在邮箱网页设置中开启 SMTP 服务，取得客户端授权码，并确认 SMTP 地址、端口和加密方式。授权码通常不是网页登录密码。服务器需要能连接邮箱的 SMTP 端口，不需要开放入站端口。

### 2. 创建配置

下面是网易 163 的示例。先把 `your-name@163.com` 改成发件邮箱，把 `recipient@example.com` 改成收件邮箱（可以与发件邮箱相同），再执行。其他邮箱按服务商参数修改 `host`、`port` 和 `security`。

```bash
mail_dir="${XDG_CONFIG_HOME:-$HOME/.config}/codex24h"
mkdir -p "$mail_dir"
chmod 700 "$mail_dir"
(umask 077; cat > "$mail_dir/mail.toml" <<'EOF'
enabled = false
host = "smtp.163.com"
port = 465
security = "ssl"
from = "your-name@163.com"
to = ["recipient@example.com"]
username = "your-name@163.com"
password_file = "smtp-password"
EOF
)
chmod 600 "$mail_dir/mail.toml"
```

此步骤用于首次配置，已有配置请直接编辑，避免覆盖。默认文件是 `~/.config/codex24h/mail.toml`；设置了 `XDG_CONFIG_HOME` 时使用其下的 `codex24h/mail.toml`。

### 3. 保存授权码并测试

```bash
codex24h-mail set-password
codex24h-mail test
```

第一条命令提示后输入授权码，输入不会显示。授权码保存在配置旁的 `smtp-password` 文件中，权限为 0600，不写入命令参数或项目。

第二条命令会实际发一封测试邮件，不启动模型任务。显示 `Test email accepted by SMTP server.` 后检查收件箱及垃圾邮件；`enabled = false` 时也能测试。

### 4. 启用自动通知

确认收到测试邮件后，把配置中的 `enabled = false` 改成 `enabled = true`：

```bash
sed -i 's/^enabled = false$/enabled = true/' "${XDG_CONFIG_HOME:-$HOME/.config}/codex24h/mail.toml"
codex24h
# 或恢复原会话：codex24h resume --last
```

之后每轮回答完成自动发信；Goal 完成会特别标注。已运行的会话需退出并通过 codex24h 恢复，才能接入通知；直接运行 `codex` 不会启用 wrapper 的通知配置。也支持 `codex24h exec`。

## 模型容量不足

原生 TUI 显示 `Selected model is at capacity` 或 `Loaded model is at capacity` 的红色错误行时，自动发送类型为 **模型容量不足** 的邮件，附会话名和恢复命令。它不代表任务完成，不会标成 Goal 完成。

同一轮任务的容量错误只提示一次，重绘或重试不重复；下一轮再次遇到容量错误会重新提示。普通输入、回答中引用这些文字、全文历史浏览不会触发。该通知识别的是原生红色错误行，并非 session 错误事件；Codex 0.158.0 不保存这种事件到 session。当前只覆盖 codex24h 承载的 TUI，`exec` 的容量错误通知尚未支持。

## 其他配置

`ssl` 使用隐式 TLS，常见端口为 465；`starttls` 使用 STARTTLS，常见端口为 587。按服务商要求填写，连接会验证服务器证书，不支持明文认证。私有 SMTP 可用 `ca_file` 指定受信任的 CA 文件。

也可用 `password_env` 指定授权码环境变量，不能与 `password_file` 同时设置；变量必须在 Codex 进程环境中可用，共享后台进程推荐使用文件。

## 常见问题

- **找不到 codex24h-mail**：确认已安装新版，并将 `~/.local/bin` 加入 PATH。
- **缺少 tomllib**：运行的是旧版源码脚本，执行上面的一行命令更新到预编译 Release，无需升级系统 Python。
- **认证失败 / SMTPAuthenticationError**：检查 SMTP 是否开启，使用客户端授权码，并确认 `username` 与发件邮箱。
- **TimeoutError / 连接失败**：检查服务器出站防火墙、云服务商 SMTP 限制及端口是否与加密方式对应。
- **测试成功但自动通知没有发送**：确认 `enabled = true`，同一服务器用户通过 codex24h 新启动或恢复会话，且未设置 `CODEX24H_MAIL=0`；再用下面的 `status` 检查。

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
