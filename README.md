# Codex24h

<img src="assets/huangdou.png" alt="戴墨镜、竖起大拇指的黄豆人" width="180" align="right" />

> 用刷短视频的时间刷会 codex

得益于科技的发展和 AI 的进步，现在您可以随时随地上班，无论是在外业务、组间休息还是三更起夜，您都可以拿出您的手机，连接终端查看您的好爱棒 codex 把活干得怎么样了。

不过终端上的使用体验并谈不上好：一是没法正常上划，二是历史不全，三是输入不便。所以您需要 codex24h，虽然不过是一个套在 codex 外面的 TUI，却极大改善了上述问题带来的不便。

这样一来，下班时间也终于在科技进步中重新获得了生产资料属性，真是可喜可贺，可喜可贺。

<br clear="right" />

## 它做了什么

直接承载本机原版 Codex 的轻量 PTY wrapper。上滚后冻结阅读画面，Codex 在后台继续运行；回到底部恢复跟随。输入框、slash commands、模型、approval、diff、MCP 和会话恢复仍由原版 Codex 提供。

不调用 app-server API，不复制认证，不更改 Codex 全局配置，也不创建容器或虚拟机。原版 Codex 自己是否使用后台 daemon，仍由它的参数和设置决定。

## 安装与使用

已安装 Rust 和 Codex 的 Linux / WSL 环境：

```bash
git clone https://github.com/Vinnish-A/codex24h.git
cd codex24h
./install.sh
codex24h
codex24h resume
codex24h resume --last
codex24h resume SESSION_ID
codex24h -C /path/to/project
codex24h exec --json "your prompt"
```

安装位置默认 `~/.local/bin/codex24h`，入口统一为 `codex24h`。可用 `CODEX24H_BIN_DIR` 选择安装目录。原来的 `codex` 命令保持不变。

`codex24h --help` 显示原版 Codex 帮助。Wrapper 的帮助在界面中按 **Ctrl+]，然后 ?**。所有原始参数按字节保留；交互入口增加 `--no-alt-screen`，由 wrapper 管理外层屏幕和历史。非交互命令、帮助和非 TTY 输入/输出直接执行原版 Codex，保留 stdout、stderr 和退出码。

## 滚动与输入

| 操作 | 行为 |
|---|---|
| 鼠标滚轮、PageUp / PageDown | 浏览历史；下滚到底部恢复跟随 |
| Ctrl+]，然后 b | 立即回到最新内容 |
| Ctrl+]，然后 / | 搜索历史 |
| Ctrl+]，然后 [ | 进入复制模式 |
| Ctrl+]，然后 ? | Wrapper 帮助 |
| Ctrl+]，然后 p，再按一键 | 将下一键原样发送给 Codex，包括被 wrapper 占用的翻页键 |
| Ctrl+]，然后 Ctrl+] | 发送原始 Ctrl+] |
| 点击底部 wrapper 状态条 | 回到最新内容 |

浏览状态下普通输入仍发送给 Codex，但不会自动返回底部。显式进入搜索、复制或帮助模式后，相关按键由 wrapper 处理。粘贴内容不会触发 wrapper 快捷键。

滚轮浏览聊天记录；实时输入框中的上下方向键和 Tab 保持 Codex 原生的历史输入与补全行为。普通点击不会进入复制模式；需要拖选时先按 Ctrl+]，然后 [。浏览历史后按 Ctrl+]，然后 b 回到实时输入框。

模型发起的选项问题由原版 Codex 显示并处理，方向键、Tab、Enter、Escape 和补充文字照常使用。浏览历史时出现新问题不会强制跳转；按 Ctrl+]，然后 b 查看并作答。搜索/复制模式的本地按键不会提交给问题菜单。Wrapper 不分析业务文本，因此未读提示不会额外识别“等待回答”。

`↓ N new updates` 统计冻结期间有单元格内容变化的观察帧数，不是消息数、token 数或 PTY 数据包数。后台原位重绘、清屏和历史淘汰均不覆盖当前冻结画面。

Codex 在浏览期间退出时，历史界面仍保留，可继续阅读；按 q 或 Enter 关闭。关闭后返回 Codex 的退出码，并将最后的实时屏幕文本留在外层终端，便于看到原版退出信息。

## 搜索与复制

搜索为区分大小写的字面文本匹配，支持中文与终端自动折行后的连续文本。输入查询后 Enter 转入复制模式，n / N 查找下一个或上一个匹配。

复制模式中：方向键或 hjkl 移动，v 开始/结束选择，y 或 Enter 复制，g / G 到快照首尾，e 导出。也可以点击并拖动选择。Esc 返回冻结浏览，Ctrl+] b 返回实时画面。

复制使用终端 OSC52，是否写入本地剪贴板由终端/SSH 客户端控制。客户端不支持时，按 e 导出 UTF-8 文本：有选择时导出选择内容，否则导出整个历史快照。默认导出到 `~/.cache/codex24h/`（尊重 `XDG_CACHE_HOME`），文件权限 0600；状态条显示路径。

搜索和复制入口会保存一个独立历史快照，避免后台输出移动选择或搜索结果；返回实时画面后释放。只有这类显式历史操作会复制整个保留历史，普通上滚只保存当前一屏。

## 手机 SSH

能把手势转换为远端鼠标滚轮事件的客户端可直接滑动浏览。若客户端只滚动自己的本地缓冲区，远端程序收不到手势；请用客户端软键盘的 PageUp / PageDown 或 Ctrl+] 前缀操作。Wrapper 不把客户端本地滑动伪装成已接收的远端事件。

终端缩放会传给 Codex；浏览中的冻结内容不会被实时输出替换。Unix 作业挂起/恢复会保存与还原终端模式。

## 配置

Wrapper 选项通过环境变量设置，避免与 Codex 参数重名。

| 变量 | 默认 | 作用 |
|---|---|---|
| `CODEX24H_CODEX` | PATH 中的 codex | 指定原版可执行文件，拒绝递归启动自己 |
| `CODEX24H_HISTORY` | 10000 | 保留的终端历史行数，范围 1–1000000 |
| `CODEX24H_ESCAPE_MS` | 100 | 传统终端中单独 Escape 的等待时间，范围 10–2000 ms |
| `CODEX24H_EXPORT_DIR` | XDG 缓存目录下的 codex24h | 显式文本导出位置 |

历史空间随列宽增长。本机构建的基础单元格为 24 字节，120 列 × 10000 行仅单元格主体约 27.5 MiB；100000 行约 275 MiB，附加 Unicode/样式和快照另计。默认不记录 PTY 原始数据，不自动导出会话。

`resume` 由 Codex 直接处理。Wrapper 的历史仅包含本次收到并保留的终端内容；Codex 未输出的旧会话内容不会自动补齐。原位覆盖的每一帧也不会被无限追加成“聊天记录”。

## 实现与验证

Rust、portable-pty 0.9.0、alacritty_terminal 0.26.0。一个非阻塞 poll 循环同时处理 PTY、输入、终端输出和信号；慢速外层输出不阻止后台 PTY 解析。正常画面最多 30 fps，冻结状态的后台提示最多 4 fps，用户导航及时响应。

内层是原生 Codex inline TUI，外层是一块虚拟终端画面和固定的一行状态条。协议查询由内层虚拟终端应答；不解析聊天角色、业务文本或 slash command 输出来重建 UI。

```bash
cargo test --locked
cargo build --locked
python3 tests/e2e.py

# 本机原生 Codex 冒烟测试；使用已有登录，不发送模型任务
cargo run --example native_smoke

# 宣告 Kitty 支持的兼容测试（按键仍使用传统编码）
CODEX24H_SMOKE_KITTY=1 cargo run --example native_smoke

# 真实模型提问测试：创建测试会话，会产生一次模型任务
CODEX24H_SMOKE_QUESTION=1 cargo run --example native_smoke
```

`tests/fake_codex.py` 是可控终端程序，只用于重现高速输出、协议应答、原位重绘、清屏、resize、信号和输入边界。测试程序中的界面文字断言不属于 wrapper 实现。

本项目的验证目标是 Linux / 当前 WSL 本机环境。未在实体手机或 macOS 上运行过的组合，不应仅凭模拟事件测试宣称已经实机验证。
