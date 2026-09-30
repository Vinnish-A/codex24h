# 使用说明

直接承载本机原版 Codex 的轻量 PTY wrapper。上滚后冻结阅读画面，Codex 在后台继续运行；回到底部恢复跟随。输入框、slash commands、模型、approval、diff、MCP 和会话恢复仍由原版 Codex 提供。

不调用 app-server API，不复制认证，不更改 Codex 全局配置，也不创建容器或虚拟机。原版 Codex 自己是否使用后台 daemon，仍由它的参数和设置决定。

## 安装与使用

Linux x86_64 / WSL（glibc 2.35+）上已安装并登录 Codex。安装和更新下载预编译 Release，不需要 Rust、Python 或 pip：

```bash
curl -fsSL https://raw.githubusercontent.com/Vinnish-A/codex24h/main/install.sh | bash
codex24h
codex24h resume
codex24h resume --last
codex24h resume SESSION_ID
codex24h -C /path/to/project
codex24h exec --json "your prompt"
```

预编译包包含主程序及辅助功能共用的运行时，下载后校验 SHA-256。临时文件自动清理；旧版本只在仍有进程使用时保留，下次更新清理。`CODEX24H_VERSION=v0.2.0` 可指定版本。下载超时会退出并清理，不无限等待。

网络较慢时，可先下载 Release 的 tar.gz 及同名 `.sha256` 校验文件，再用 `CODEX24H_ARCHIVE=/path/to/codex24h-linux-x86_64.tar.gz bash install.sh` 安装本地包。仍执行校验，不编译。

开发者从源码构建仍需 Rust 和 Python，发布构建见 [packaging](../packaging/README.md)。

安装位置默认 `~/.local/bin/codex24h`，入口统一为 `codex24h`。可用 `CODEX24H_BIN_DIR` 选择安装目录。原来的 `codex` 命令保持不变。

`codex24h --help` 显示原版 Codex 帮助。Wrapper 的帮助在界面中按 **Ctrl+]，然后 ?**。除实验性的 [`attach`](attach.md) 子命令外，所有原始参数按字节保留；交互入口增加 `--no-alt-screen`，由 wrapper 管理外层屏幕和历史。非交互命令、帮助和非 TTY 输入/输出直接执行原版 Codex，保留 stdout、stderr 和退出码。若提示词本身是 `attach`，可用 `codex24h -- attach`。

## 滚动与输入

| 操作 | 行为 |
|---|---|
| 鼠标滚轮、不带修饰键的 PageUp / PageDown | 浏览历史；下滚到底部恢复跟随 |
| Ctrl+]，然后 i / + / - | 固定输入区开关 / 增高两行 / 降低两行 |
| Ctrl+]，然后 b | 立即回到最新内容 |
| Ctrl+]，然后 / | 搜索历史 |
| Ctrl+]，然后 ? | Wrapper 帮助 |
| Ctrl+]，然后 p，再按一键 | 将下一键原样发送给 Codex，包括被 wrapper 占用的翻页键 |
| Ctrl+]，然后 Ctrl+] | 发送原始 Ctrl+] |
| 点击底部 wrapper 状态条 | 回到最新内容 |

浏览时默认将光标附近的 6 行原生画面固定在下方，上方历史保持冻结。输入、补全仍交给 Codex，输入光标随实时区域显示；滚轮仍只浏览历史。鼠标点击实时区域时转换回原生坐标，历史区域的点击不发送给应用。

这里固定的是终端单元格，不解析输入框或菜单的业务文本。可见光标用于定位；光标隐藏时使用底部区域。多行草稿、选项或审批对话较高时，用 **Ctrl+] 然后 +** 增大显示区域，或用 **Ctrl+] b** 回到完整实时界面。最多保留两行历史和分隔线；极小窗口会暂时隐藏实时区。搜索和帮助模式保持原来的专用视图。

带 Shift / Ctrl / Alt 的翻页组合键原样交给 Codex。浏览状态下普通输入仍发送给 Codex，但不会自动返回底部。显式进入搜索或帮助模式后，相关按键由 wrapper 处理。粘贴内容不会触发 wrapper 快捷键。

滚轮浏览聊天记录；实时输入框中的上下方向键和 Tab 保持 Codex 原生的历史输入与补全行为。文字划选和复制交给客户端，按住 Shift 拖选，再按 Ctrl+Shift+C。浏览历史时按一次 Ctrl+C 或 Ctrl+]，然后 b 回到实时输入框；回到实时界面后 Ctrl+C 恢复 Codex 原生行为。

模型发起的选项问题由原版 Codex 显示并处理，方向键、Tab、Enter、Escape 和补充文字照常使用。浏览历史时出现新问题不会强制跳转；按 Ctrl+]，然后 b 查看并作答。搜索模式的本地按键不会提交给问题菜单。Wrapper 不分析业务文本，因此未读提示不会额外识别“等待回答”。

`↓ N new updates` 统计冻结期间有单元格内容变化的观察帧数，不是消息数、token 数或 PTY 数据包数。后台原位重绘、清屏和历史淘汰均不覆盖当前冻结画面。

Codex 在浏览期间退出时，历史界面仍保留，可继续阅读；按 q 或 Enter 关闭。关闭后返回 Codex 的退出码，并将最后的实时屏幕文本留在外层终端，便于看到原版退出信息。

## 搜索与文字复制

搜索为区分大小写的字面文本匹配，支持中文与终端自动折行后的连续文本。输入查询后 Enter 查看结果，n / N 查找下一个或上一个匹配，方向键或 hjkl 移动。Esc 返回冻结浏览，Ctrl+] b 返回实时画面。搜索保存独立历史快照，避免后台输出移动结果，回到底部后释放。

文字复制由 SSH 客户端处理：按住 Shift 拖选，再按 Ctrl+Shift+C（以客户端配置为准）。wrapper 不创建选区、不自动复制、不显示复制发送提示，也不提供复制模式或服务器文件导出。图片粘贴插件不参与文字复制。原版 Codex 自己发起的终端剪贴板协议请求仍按原生行为透传。

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

历史空间随列宽增长。本机构建的基础单元格为 24 字节，120 列 × 10000 行仅单元格主体约 27.5 MiB；100000 行约 275 MiB，附加 Unicode/样式和快照另计。不记录 PTY 原始数据，不导出终端历史到服务器文件。

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

滚轮浏览需要终端开启鼠标上报，因此一般需要按住 Shift 使用客户端原生拖选，再按 Ctrl+Shift+C 复制。拖选、选区显示和复制全部由客户端处理；wrapper 不会因拖选进入冻结模式或自动写入剪贴板。子应用启用鼠标时，实时区域的普通鼠标事件仍透传。

Shift+方向键的发送方式由客户端映射决定，不会把未收到的按键猜测成另一种按键。

### 固定区的动态范围

`Ctrl+] + / -` 调整的是最大高度。输入光标可见时围绕光标显示；原生菜单隐藏光标时，围绕反显选项及相邻行显示，方向键改变选项后视图跟随。交互区由终端空行分隔限制，内容较少时不会为填满高度而不断向上纳入命令输出。此判断只读取 VT 光标、反显属性和空行；没有反显、没有空行分隔的特殊界面只能回退，不能保证识别任意程序的语义焦点。

调整固定区高度或开关时会退出搜索模式，恢复浏览与实时输入，状态栏显示最大行数。跟随模式中调整只设置上翻后的高度；按键顺序是 Ctrl+]，松开，再按 +（或 =）/ -。

历史浏览会明确显示 HISTORY (frozen)，搜索结果显示 SEARCH RESULTS。这些状态冻结的是显示，子进程继续运行。普通拖选不会改变 wrapper 的运行模式。

右键属于 SSH 客户端。Tabby 图片插件 0.1.6 会在 SSH 标签页隐藏原有 Export to file 菜单项（该项原本保存到 Windows 本机），保留右键菜单/粘贴设置；菜单粘贴、自定义粘贴快捷键与 Ctrl+V 使用同一图片上传逻辑。其他客户端若自行拦截鼠标，使用 Shift+右键打开本地菜单。


## 当前 session 请求导航

按 `Ctrl+]`，松开后按 `r`。列表按发送顺序排列，默认选中最新请求。

- `↑ / ↓`、滚轮、PageUp / PageDown 选择，Home / End 到首尾。
- 输入关键词筛选；Backspace 删除，Ctrl+U 清空；Enter 跳到原生终端历史中的请求。
- 跳转后保持原生颜色、换行和布局，可继续滚动；固定输入区仍可输入、补全。
- 列表中 Esc / Ctrl+C 取消并恢复原视图；浏览记录时 Ctrl+C 或 `Ctrl+] b` 返回实时 Codex。

预编译包已包含辅助程序。只读当前进程的 session 文件以列出用户请求；不显示回答或工具调用的原始 JSON，不生成另一套聊天阅读器。请求文字用于在 VT 终端历史中定位，跳转后显示保存的终端单元格，后台输出不会拉走视图。

当前缓冲能准确定位的请求直接跳到原生终端记录。`resume`、上下文压缩或缓冲淘汰后的旧请求，转交 Codex 自带的 full transcript 搜索，列表标为 `[full history]`；不再把“尚未显示在缓冲里”标为不可用。

原生全文历史以请求中的一小段正文作为搜索词；长粘贴会跳过 `Q:` 等短标题。定位后直接用滚轮、↑↓、PageUp / PageDown 阅读，不需要先关搜索；查询未完成时会暂存滚动操作。Ctrl+C、Esc、q 或 Ctrl+] b 返回输入框，保留草稿；退出期间显示 closing，并在原生界面确认关闭前接住重复退出键；Ctrl+] r 可重新打开请求列表。普通历史浏览中 Ctrl+P / Enter 切换上一条 / 下一条请求，状态栏显示请求序号及首尾边界。按 / 进入关键词搜索后，Ctrl+P / Enter 切换上一处 / 下一处匹配；滚动后仍保持匹配导航，没有更多匹配时会明确提示（编辑搜索词时 q 是普通文字）。Home / End 跳到原生历史首尾。正文重复或被其他消息引用时可切换匹配，不保证对应唯一的消息 ID。

已在 Codex 0.158.0 上验证。wrapper 等待原生 transcript 和 Find 界面出现后才填入搜索词，不发送提交任务的 Enter；打开失败会提示。识别的只是固定界面标记，业务内容不由 wrapper 重画。该路径需要 Codex 本身支持全文历史搜索；已删除的 session 日志仍无法恢复。
