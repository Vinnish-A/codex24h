# 使用说明

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

`codex24h --help` 显示原版 Codex 帮助。Wrapper 的帮助在界面中按 **Ctrl+]，然后 ?**。除实验性的 [`attach`](attach.md) 子命令外，所有原始参数按字节保留；交互入口增加 `--no-alt-screen`，由 wrapper 管理外层屏幕和历史。非交互命令、帮助和非 TTY 输入/输出直接执行原版 Codex，保留 stdout、stderr 和退出码。若提示词本身是 `attach`，可用 `codex24h -- attach`。

## 滚动与输入

| 操作 | 行为 |
|---|---|
| 鼠标滚轮、不带修饰键的 PageUp / PageDown | 浏览历史；下滚到底部恢复跟随 |
| Ctrl+]，然后 i / + / - | 固定输入区开关 / 增高两行 / 降低两行 |
| Ctrl+]，然后 b | 立即回到最新内容 |
| Ctrl+]，然后 / | 搜索历史 |
| Ctrl+]，然后 [ | 进入复制模式 |
| Ctrl+]，然后 ? | Wrapper 帮助 |
| Ctrl+]，然后 p，再按一键 | 将下一键原样发送给 Codex，包括被 wrapper 占用的翻页键 |
| Ctrl+]，然后 Ctrl+] | 发送原始 Ctrl+] |
| 点击底部 wrapper 状态条 | 回到最新内容 |

浏览时默认将光标附近的 6 行原生画面固定在下方，上方历史保持冻结。输入、补全仍交给 Codex，输入光标随实时区域显示；滚轮仍只浏览历史。鼠标点击实时区域时转换回原生坐标，历史区域的点击不发送给应用。

这里固定的是终端单元格，不解析输入框或菜单的业务文本。可见光标用于定位；光标隐藏时使用底部区域。多行草稿、选项或审批对话较高时，用 **Ctrl+] 然后 +** 增大显示区域，或用 **Ctrl+] b** 回到完整实时界面。最多保留两行历史和分隔线；极小窗口会暂时隐藏实时区。搜索、复制和帮助模式保持原来的专用视图。

带 Shift / Ctrl / Alt 的翻页组合键原样交给 Codex。浏览状态下普通输入仍发送给 Codex，但不会自动返回底部。显式进入搜索、复制或帮助模式后，相关按键由 wrapper 处理。粘贴内容不会触发 wrapper 快捷键。

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
| `CODEX24H_PIN_ROWS` | 6 | 浏览时实时输入区行数，0 关闭，范围 0–100 |
| `CODEX24H_HISTORY` | 10000 | 保留的终端历史行数，范围 1–1000000 |
| `CODEX24H_ESCAPE_MS` | 100 | 传统终端中单独 Escape 的等待时间，范围 10–2000 ms |
| `CODEX24H_EXPORT_DIR` | XDG 缓存目录下的 codex24h | 显式文本导出位置 |

历史空间随列宽增长。本机构建的基础单元格为 24 字节，120 列 × 10000 行仅单元格主体约 27.5 MiB；100000 行约 275 MiB，附加 Unicode/样式和快照另计。默认不记录 PTY 原始数据，不自动导出会话。

`resume` 由 Codex 直接处理。Wrapper 的历史仅包含本次收到并保留的终端内容；Codex 未输出的旧会话内容不会自动补齐。原位覆盖的每一帧也不会被无限追加成“聊天记录”。

## 实现与验证

### SSH 图片粘贴边界

当前没有实现 Windows 剪贴板图片经 SSH 的零配置转发。服务器上的 wrapper 需要终端客户端主动上传图片或实现图片剪贴板协议，不能仅凭一次粘贴按键读取连接电脑的图片。

截至 2026-09-29，Xshell [官方说明](https://netsarang.atlassian.net/wiki/spaces/ENSUP/pages/2237304414/Terminal%2B_%2BAdvanced)中的 OSC 52 是向 Windows 剪贴板复制文本；Termius [桌面更新记录](https://docs.termius.com/changelog/desktop)未找到图片粘贴支持依据。Termius [iOS / iPadOS 文档](https://docs.termius.com/terminal/mobile-terminal#paste-images-and-files)则明确支持后台 SFTP 上传到 `/tmp` 并插入远端路径，但不能据此推断 Windows 版也支持，且本项目尚未实机验证该移动端链路。

本机 Xshell 8.0.0110 已实测：图片剪贴板的 Ctrl+Shift+V / Shift+Insert，以及复制 PNG 文件后的 Ctrl+Shift+V，均未向 SSH 接收端插入图片路径；前后文字粘贴对照正常。这些默认粘贴方式不满足自动图片上传要求。详细过程见 [TESTING.md](../TESTING.md)。

Windows Tabby 1.0.237 + [SSH Image Paste 插件](https://github.com/Vinnish-A/tabby-ssh-image-paste)已验证原生 SSH 图片上传。插件 0.1.5 支持 Ctrl+V / Ctrl+Shift+V，并用 bracketed paste 将图片路径送进 Codex；上传失败显示错误。下载并双击 install.cmd 安装，再完全退出并重新打开 Tabby。后续双击同一个脚本更新；插件不再启动时自动检查或下载更新。支持 HTML 整段图文、多张图片及段落顺序。请使用“配置和连接”中的原生 SSH 连接；在 PowerShell 里运行 ssh 不提供插件所需的 SFTP 会话。

### 终端实现

Rust、portable-pty 0.9.0、alacritty_terminal 0.26.0。一个非阻塞 poll 循环同时处理 PTY、输入、终端输出和信号；慢速外层输出不阻止后台 PTY 解析。正常画面和固定输入区最多 30 fps；关闭实时区时，冻结状态的后台提示最多 4 fps，用户导航及时响应。

内层是原生 Codex inline TUI，外层是一块虚拟终端画面、可选的原生实时区域和一行状态条。协议查询由内层虚拟终端应答；不解析聊天角色、业务文本或 slash command 输出来重建 UI。

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


## 会话被另一个应用占用

Codex 0.158.0 的 `This conversation is open in another app` 表示它没有取得会话写入锁。原进程退出后可按 `R` 重试；单纯重试不会终止还活着的写入者。不要删除 `thread-writer-locks` 中的文件，删除文件可能让两个进程分别持有不同 inode 的锁。

```bash
codex24h session <完整-session-UUID>             # 查询内核锁对应的 PID / tmux 窗格
codex24h session <完整-session-UUID> --attach    # 回到原 tmux 会话，不启动第二个 Codex
codex24h session <完整-session-UUID> --takeover  # 终止唯一独立写入者，再以 --no-daemon resume
```

从另一个终端执行接管。只处理当前用户、已核实持锁的单会话 Codex，使用 pidfd 防止 PID 复用误杀，发送 SIGTERM 后最多等 5 秒；不删除锁、不强杀共享 app-server，也不会终止调用命令自身的父 Codex。此入口需要 Linux、Python 3，接管还需要 pidfd API 和支持 `--no-daemon` 的 Codex（已测 0.158.0）。Conda Python 缺少 pidfd 时会尝试系统 `/usr/bin/python3`。

如果占用者是共享 app-server，查询会明确指出；应在原应用关闭该会话。该命令不会为了释放一个 session 终止其他会话。`--attach --socket <路径>` 可选择其他 tmux server。查不到本机锁不能排除其他主机或其他 Codex home 中的占用。

## 鼠标拖选

普通界面可直接左键拖选，移动后才进入复制模式；单击仍保留原生输入框、方向键和 Tab 行为。拖选画面取自按下鼠标时的快照，不受后续输出影响，松开时通过 OSC52 发送所选文本到客户端剪贴板。按 `Esc` 回到浏览，`Ctrl+] b` 回到最新内容。客户端需允许 OSC52；不支持时可按 `e` 导出文字，或按住 `Shift` 用终端自身的鼠标选择和复制。

如果子应用开启了鼠标交互，实时区域仍把鼠标交给子应用；历史区域和显式复制模式仍可选取。Xshell 中 Shift+拖动用于本地选择；Shift+方向键的发送方式由客户端映射决定，不会把未收到的按键猜测成另一种按键。


### 固定区的动态范围

`Ctrl+] + / -` 调整的是最大高度。输入光标可见时围绕光标显示；原生菜单隐藏光标时，围绕反显选项及相邻行显示，方向键改变选项后视图跟随。交互区由终端空行分隔限制，内容较少时不会为填满高度而不断向上纳入命令输出。此判断只读取 VT 光标、反显属性和空行；没有反显、没有空行分隔的特殊界面只能回退，不能保证识别任意程序的语义焦点。

调整固定区高度或开关时会退出复制/搜索模式，恢复浏览与实时输入，状态栏显示最大行数。跟随模式中调整只设置上翻后的高度；按键顺序是 Ctrl+]，松开，再按 +（或 =）/ -。
