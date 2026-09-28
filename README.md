# Codex24h

给 Codex 套一个能安心翻记录的壳。你往上翻，它继续干活，谁也别拽谁。

用原来的 Codex、原来的登录。模型、审批、补全照旧，命令名换成 `codex24h` 就行。

## 装上

Linux / WSL，先装好 Rust 和 Codex：

```bash
git clone https://github.com/Vinnish-A/codex24h.git
cd codex24h
./install.sh
```

## 开用

```bash
codex24h
codex24h resume
codex24h resume --last
```

原来传给 `codex` 的参数，照传。

| 想干什么 | 怎么按 |
|---|---|
| 翻聊天记录 | 滚轮 / PageUp / PageDown |
| 回到最新 | 下滚到底，或 Ctrl+] 然后 b |
| 找以前发过的话 | 输入框里按 ↑ / ↓ |
| 原生补全 | Tab |
| 搜记录 | Ctrl+] 然后 / |
| 复制 | Ctrl+] 然后 [，选中后按 y |
| 看帮助 | Ctrl+] 然后 ? |

手机滑不动，就用终端软键盘翻页；手势得靠 SSH 客户端配合。`resume` 能翻到多少旧记录，取决于 Codex 本次吐出来多少，目前不会自动补齐整段历史。

更多见[使用说明](docs/usage.md)和[测试记录](TESTING.md)。

## 说两句

<img src="assets/huangdou.png" alt="戴墨镜、竖起大拇指的黄豆人" width="180" align="right" />

> 用刷短视频的时间刷会 codex

得益于科技的发展和 AI 的进步，现在您可以随时随地上班，无论是在外业务、组间休息还是三更起夜，您都可以拿出您的手机，连接终端查看您的好爱棒 codex 把活干得怎么样了。

不过终端上的使用体验并谈不上好：一是没法正常上划，二是历史不全，三是输入不便。所以您需要 codex24h，虽然不过是一个套在 codex 外面的 TUI，却极大改善了上述问题带来的不便。

这样一来，下班时间也终于在科技进步中重新获得了生产资料属性，真是可喜可贺，可喜可贺。

<br clear="right" />
