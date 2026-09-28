# Codex24h

<img src="assets/huangdou.png" alt="黄豆人" width="180" align="right" />

> 用刷短视频的时间刷会 codex

Codex CLI 的轻量终端包装器，提供稳定的滚动和历史浏览。上翻后画面保持不动，Codex 在后台继续输出；滚动到底部后恢复跟随。

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
| 搜索历史 | Ctrl+] 然后 / |
| 复制 | Ctrl+] 然后 [，选中后按 y |
| 帮助 | Ctrl+] 然后 ? |

手机触控滚动需要 SSH 客户端支持发送滚轮事件，否则可使用软键盘翻页。`resume` 的可浏览历史仅包含 Codex 本次实际输出的内容，暂不支持完整历史补齐。

详细配置见[使用说明](docs/usage.md)，验证范围见[测试记录](TESTING.md)。

## 题外话

得益于科技的发展和 AI 的进步，现在您可以随时随地上班，无论是在外业务、组间休息还是三更起夜，您都可以拿出您的手机，连接终端查看您的好爱棒 codex 把活干得怎么样了。

不过终端上的使用体验并谈不上好：一是没法正常上划，二是历史不全，三是输入不便。所以您需要 codex24h，虽然不过是一个套在 codex 外面的 TUI，却极大改善了上述问题带来的不便。

这样一来，下班时间也终于在科技进步中重新获得了生产资料属性，真是可喜可贺，可喜可贺。
