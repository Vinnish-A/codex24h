# Codex24h

<img src="assets/huangdou.png" alt="黄豆人" width="180" align="right" />

> 用刷短视频的时间刷会 codex

Codex CLI 的轻量终端包装器，提供稳定的滚动和历史浏览。上翻后历史保持不动，下方保留实时输入区，Codex 在后台继续输出；滚动到底部后恢复跟随。

使用本机 Codex 和现有登录，保留原生输入框、补全、模型选择、审批等交互。

<br clear="right" />

## 安装

支持 Linux / WSL，需要已安装 Rust 和 Codex。

```bash
git clone https://github.com/Vinnish-A/codex24h.git
cd codex24h
./install.sh
```

默认安装到 `~/.local/bin/codex24h`。

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
| 返回最新内容 | 下滚到底部，或 Ctrl+] 然后 b |
| 召回历史输入 | 输入框中的 ↑ / ↓ |
| 原生补全 | Tab |
| 固定输入区开关 / 调整高度 | Ctrl+] 然后 i / + / - |
| 搜索历史 | Ctrl+] 然后 / |
| 复制 | 鼠标拖选后松开；或 Ctrl+] 然后 [，选中后按 y |
| 帮助 | Ctrl+] 然后 ? |

手机触控滚动需要 SSH 客户端支持发送滚轮事件，否则可使用软键盘翻页。`resume` 的可浏览历史仅包含 Codex 本次实际输出的内容，暂不支持完整历史补齐。

已经在 tmux 单窗格窗口里运行的 Codex，可以从另一个终端接入，不必重启：

```bash
codex24h attach --list
codex24h attach <PID>
```

按 Ctrl+] 然后 d 断开，原任务继续运行。需要 tmux、Python 3 和 `tic`。**全屏模式下可能只有画面冻结，没有可翻的聊天历史**，`--list` 会标注；普通终端进程暂不支持接入。新会话仍建议直接用 `codex24h`。详见[接入说明](docs/attach.md)。

## 图片粘贴

通过 SSH 使用时，建议选择 [Tabby](https://tabby.sh)，并安装 [tabby-ssh-image-paste](https://github.com/Vinnish-A/tabby-ssh-image-paste)。Windows 上复制截图后，在远程 Codex 输入框按 `Ctrl+V` 或 `Ctrl+Shift+V`，插件会通过当前连接的 SFTP 上传图片并填入路径，供 Codex 识别为附件。

服务器需支持 SFTP，且 `/tmp` 可写；无需额外登录或服务端插件。插件尚未上架商店，按其 [安装说明](https://github.com/Vinnish-A/tabby-ssh-image-paste#安装到-windows-tabby) 下载 ZIP，解压后双击 `install.cmd`，再重启 Tabby。插件默认自动检查 GitHub 更新，可在设置中关闭。

## 邮件通知

可在每轮回答完成后自动发送邮件，附上会话名和恢复命令。Goal 完成时会特别标注。由本机程序触发，不需要 agent 介入。

需要配置 SMTP 邮箱，见[邮件通知设置](docs/mail.md)。

详细配置见[使用说明](docs/usage.md)，验证范围见[测试记录](TESTING.md)。

## 题外话

得益于科技的发展和 AI 的进步，现在您可以随时随地上班，无论是在外业务、组间休息还是三更起夜，您都可以拿出您的手机，连接终端查看您的好爱棒 codex 把活干得怎么样了。

不过终端上的使用体验并谈不上好：一是没法正常上划，二是历史不全，三是输入不便。所以您需要 codex24h，虽然不过是一个套在 codex 外面的 TUI，却极大改善了上述问题带来的不便。

这样一来，下班时间也终于在科技进步中重新获得了生产资料属性，真是可喜可贺，可喜可贺。
