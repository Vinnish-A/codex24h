# Codex24h

<img src="assets/huangdou.png" alt="黄豆人" width="180" align="right" />

> 用刷短视频的时间刷会 codex

Codex CLI 的轻量终端包装器，提供稳定的滚动和历史浏览。上翻后历史保持不动，下方保留实时输入区，Codex 在后台继续输出；滚动到底部后恢复跟随。

使用本机 Codex 和现有登录，保留原生输入框、补全、模型选择、审批等交互。

<br clear="right" />

## 安装 / 更新

支持 Linux / WSL，需要已安装 Rust、Python 3、curl 和 Codex。安装和更新使用同一条命令：

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
```

默认安装到 `~/.local/bin`。下载和编译在临时目录完成，结束后自动清理；保留 Cargo 共用的依赖缓存。

## 使用

用法与 `codex` 相同：

```bash
codex24h
codex24h resume
codex24h resume --last
```

| 操作 | 按键 |
|---|---|
| 浏览聊天记录 | 滚轮 / PageUp / PageDown |
| 返回最新内容 | 浏览时 Ctrl+C，或 Ctrl+] 然后 b |
| 召回历史输入 | 输入框中的 ↑ / ↓ |
| 原生补全 | Tab |
| 固定输入区开关 / 调整高度 | Ctrl+] 然后 i / + / - |
| 搜索历史 | Ctrl+] 然后 / |
| 当前 session 请求列表 | Ctrl+] 然后 r |
| 复制 | 按住 Shift 用终端划选，再按 Ctrl+Shift+C |
| 帮助 | Ctrl+] 然后 ? |

划选和复制由终端处理：按住 Shift 拖选，再按 Ctrl+Shift+C（以客户端设置为准）。手机滑屏需要 SSH 客户端支持发送滚轮事件。

### 跳转到之前的请求

按 **Ctrl+] 然后 r** 打开当前会话的请求列表，↑↓ 选择，输入关键词筛选，Enter 跳转，Esc 取消。

普通滚动保留本次终端收到的内容。更早的请求标为 `[full history]`，通过 Codex 原生全文历史打开，无需重新登录或发送模型请求。

在全文历史中：

- 滚轮 / PageUp / PageDown 阅读上下文。
- Ctrl+P / Enter 跳到上一条 / 下一条请求。
- 按 `/` 搜索后，Ctrl+P / Enter 改为跳到上一个 / 下一个匹配。
- Ctrl+C / Esc 返回输入框。

### 接入正在运行的 Codex

已经在 tmux 单窗格窗口里运行的 Codex，可以从另一个终端接入，不必重启：

```bash
codex24h attach --list
codex24h attach <PID>
```

按 Ctrl+] 然后 d 断开，任务继续运行。需要 tmux、Python 3 和 `tic`；仅支持 tmux 单窗格窗口，全屏模式可能没有可翻阅的历史，见[接入说明](docs/attach.md)。

## 图片粘贴

Windows SSH 用户建议使用 [Tabby](https://tabby.sh) 和 [tabby-ssh-image-paste](https://github.com/Vinnish-A/tabby-ssh-image-paste) 插件，支持图片及图文混合粘贴。

按[安装说明](https://github.com/Vinnish-A/tabby-ssh-image-paste#安装到-windows-tabby)下载并双击 `install.cmd`，重启 Tabby。更新也双击同一脚本，不会后台自动更新。

使用 Tabby 原生 SSH 连接，在 Codex 输入框按 Ctrl+V 或 Ctrl+Shift+V。图片通过当前连接的 SFTP 上传；服务器需支持 SFTP，且 `/tmp` 可写。

## 邮件通知

配置 SMTP 后，每轮回答完成会自动发邮件，附上会话名和恢复命令；Goal 完成会特别标注。无需 agent 介入，见[邮件通知设置](docs/mail.md)。

详细配置见[使用说明](docs/usage.md)，验证范围见[测试记录](TESTING.md)。

## 题外话

得益于科技的发展和 AI 的进步，现在您可以随时随地上班，无论是在外业务、组间休息还是三更起夜，您都可以拿出您的手机，连接终端查看您的好爱棒 codex 把活干得怎么样了。

不过终端上的使用体验并谈不上好：一是没法正常上划，二是历史不全，三是输入不便。所以您需要 codex24h，虽然不过是一个套在 codex 外面的 TUI，却极大改善了上述问题带来的不便。

这样一来，下班时间也终于在科技进步中重新获得了生产资料属性，真是可喜可贺，可喜可贺。
