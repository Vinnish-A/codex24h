# 邮件通知

启用后，每轮回答结束会自动发送一封邮件，附上会话名、Session ID、完成时间和恢复命令。若这一轮完成了 Goal，标题和正文会标注 **Goal 已完成**。发信由本机程序执行，不需要 agent 调用工具或生成邮件。

邮件标题示例：

```text
[Codex24h][本轮已完成] 整理实验数据
[Codex24h][Goal 已完成] 整理实验数据
[Codex24h][自动恢复请求已提交] 整理实验数据
[Codex24h][容量不足，请检查] 整理实验数据
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

启用邮件后，codex24h 承载的 TUI 遇到容量错误，会对每个不同失败回合**独立随机等待 3～10 秒**，然后尝试恢复：

- 普通任务提交 `go on`。
- Goal 从状态库确认身份，只在发送前仍为 `paused` 时提交 `/goal resume`。`complete`（界面 achieved）不恢复；`active`、`blocked`、用量或预算限制状态不自动改动。
- Codex 确认接收恢复请求之后才发 **自动恢复请求已提交** 邮件，注明第几次、指令及 Session ID；这表示请求已提交，不表示任务已成功完成。
- 每个会话连续自动恢复最多三次，第四次容量失败不再自动回复，发送 **容量不足，请检查**。成功完成一轮或手动提交新任务后重置；自动 `go on` 不重置，同轮重绘不重复计数。计数保存在邮件状态库，多个前端共用上限。

等待期间输入、草稿、切换会话、原生菜单或无法确认状态时不会盲发指令，会发送恢复受阻的检查通知。未确认的提交占用一次额度，避免不确定情况下反复执行。程序中断后不会重放旧恢复指令；再次打开前端时将中断记录转成检查通知。`retry` 只重试邮件，不会重新提交模型任务。

共享后台模式按本前端确认会话的结构化失败记录识别 `serverOverloaded`、原生容量错误及带 HTTP 状态前缀的容量错误，不依赖错误行颜色或前端持有日志。会话来自本前端用户消息 ID、明确恢复 UUID 和原生 `/resume` 确认。新建 Goal 的指令没有普通消息 ID，必要时在当前前端执行只读 `/status` 获取实际 Session ID，再读取 Goal 状态，不猜测最近会话。

控制信息通过私有临时目录中的原子文件传递，与 Codex 事件管道分开；不保存对话正文。发送前仍检查主会话归属，子 agent 与身份不明的会话不发信。状态库不可读会阻止自动回复；无法确认归属时也不会发送邮件，终端会报告元数据问题。监控进程退出时状态栏持续显示 `Mail / recovery unavailable`，Codex 继续运行；重新打开前端恢复监控。

独立模式保留原生完成通知，并使用结构化失败记录及本进程日志定位的原生红色容量错误接入相同恢复流程。当前恢复能力验证于 Codex 0.162.0/0.162.1 元数据；缺少可读 Goal 状态库的旧版本不会被当成普通任务盲发 `go on`。`exec` 的容量恢复和远程 app-server 尚未支持。

不会批量补发启动前的历史容量错误。更新后需新启动或使用 `codex24h resume SESSION_UUID` 恢复前端，既有前端不会自动加载新代码；不需要重启共享后台。邮件配置、授权码和队列保留。

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

入队和实际发信前均确认会话归属。子 agent、来源未知、元数据缺失或读取失败时不发信；无法确认归属的旧队列记录标为 `suppressed`，不会由 `retry` 重发。因此状态库不可用时可能漏掉主会话通知，恢复后也不会补发这些被拦截的邮件。主会话的一轮完成邮件不代表所有子 agent 都已结束。

临时关闭通知：

```bash
CODEX24H_MAIL=0 codex24h resume --last
```

长期关闭可将配置中的 `enabled` 改为 `false`。使用 `CODEX24H_MAIL_CONFIG` 指定其他配置文件；辅助命令也支持 `--config PATH`。队列默认位于 `~/.local/state/codex24h/mail/`，尊重 `XDG_STATE_HOME`，也可用配置项 `state_dir` 修改。

## 触发方式与范围

使用 Codex 原生的 [`notify` / `agent-turn-complete` 事件](https://learn.chatgpt.com/docs/config-file/config-advanced#notifications)。Wrapper 仅为本次启动附加通知配置，不修改 Codex 全局配置；原有通知命令仍会执行。共享模式通过作用域内的轮次元数据观察完成和容量事件；没有用模型判断是否发信。

会话名称、来源、父子关系和 Goal 状态从本机 Codex 元数据只读获取；版本验证范围见 [README](../README.md#codex-版本兼容)。名称优先使用手动命名，其次使用 Codex 保存的标题；名称为空时以 Session ID 标识。邮件不附带对话正文。子 agent 的独立完成事件和容量错误不发送邮件。

Goal 必须在本次接入通知之后变为 `complete` 才会标注，且每个 Goal 只标注一次；恢复一个早已完成的 Goal 不会误报为新完成。它代表 Codex 的 Goal 状态，不额外判断任务的实际质量。

本功能针对本机 Codex；远程 app-server 不在支持范围内。Codex 内部状态库的未来变更若导致无法确认主会话身份，自动通知会停止发送。

## 验证

```bash
cargo build --locked
python3 tests/mail.py
python3 tests/recovery.py

# 可选：真实 Codex + 隔离后台 + 本地模拟 SSE 容量错误及 TLS SMTP，无外部模型请求。
CODEX24H_CODEX=/path/to/codex python3 tests/native_capacity.py
CODEX24H_CODEX=/path/to/codex python3 tests/native_capacity.py --standalone

# 可选：发送两次真实模型请求，验证 exec 和 resume TUI 自动通知。
# 邮件只投递给本机测试 TLS SMTP 服务，不发到外部邮箱。
python3 tests/native_mail.py
```
