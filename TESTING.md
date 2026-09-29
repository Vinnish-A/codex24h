# 本机验证记录

2026-09-28，Linux / WSL，本机 Codex CLI 0.156.1，使用已有认证。

- `cargo test --locked`：41 项通过，覆盖输入协议、冻结视图、历史搜索/复制、终端绘制与启动参数。
- `CODEX24H_TEST_BIN=/home/vinnish/.local/bin/codex24h python3 tests/e2e.py`：10 项通过。覆盖 PTY 参数/输入/退出码、重绘期间冻结、选项与补充文字、取消、搜索/复制不误答、终端恢复、信号与挂起恢复。
- 原生 `native_smoke`：传统输入及宣告 Kitty 支持的兼容模式均通过模型菜单、冻结期间菜单更新、回到底部、MCP 页面、无仓库 diff 响应、帮助及 resume 选择器测试。后者仍使用传统按键字节，不代表完整增强键编码实测。
- 安装后的 release：真实模型 `request_user_input` 在冻结期间到达，回到底部后选择“香蕉”，模型根据实际选择输出“已选择：香蕉”。随后按测试会话 ID 恢复，回答可见。测试提示词不含预期完整回答，避免将输入回显误判为模型结果。
- release 2 MiB 连续输出测试：约 0.04 秒墙钟时间、0.02 秒 wrapper CPU、4608 KiB RSS；外层 PTY 暂不读取时内层仍可推进。此项是输出压力测试，不代表长历史内存。
- release 长历史测试：先输出 10000 行，再在上翻浏览期间输出另外 10000 行。峰值 RSS 31744 KiB；1.5 秒空闲窗口 CPU 用量低于 0.01 秒测量分辨率。
- 本机安装入口 `codex24h` 可用；`--version`、`resume --help` 透传成功。

这些结果不意味着所有平台和原生功能均逐一验证。尚未在实体手机、macOS 上实测；真实审批工具执行和有修改的仓库 diff 未纳入本轮原生冒烟。它们由同一原版 Codex PTY 承载，并没有在 wrapper 中重写。`resume` 历史仍仅限 Codex 本次实际输出的内容。手机手势需要客户端转换成远端鼠标事件。

复现命令见 README；实际模型提问测试需显式设置 `CODEX24H_SMOKE_QUESTION=1`，会产生模型请求和测试会话。

## 长会话与输入冲突修复

同日本机 Codex 升级至 0.158.0 后，在现有 tmux（TERM=screen，134×62）中恢复超过 3 MiB 的真实长会话。发现旧版普通鼠标点击会误入 COPY，从而接管方向键和 Tab；现已改为仅在显式复制模式中接受拖选。

- 更新后的已安装 release：42 项单元测试、10 项 PTY 端到端测试通过。
- `tests/native_tmux.py`：滚轮上翻得到 39 个不同历史画面，后台更新期间正文保持冻结；下滚到底部恢复跟随。
- 同一长会话中点击输入框后，Up 召回历史发送内容，Down 恢复空草稿；`/mo` 显示原生菜单，Tab 完成为 `/model`。测试清空草稿，没有提交模型请求。
- 仍可显式进入 COPY。整组本机 tmux 回归约 12.5 秒，测试窗口随后断开，不中断被恢复会话中的运行任务。

该回归注入终端滚轮事件，未模拟手机客户端的物理手势转换。修复确认了输入模式冲突，不足以证明此前那次启动卡顿的唯一原因。

```bash
python3 tests/native_tmux.py SESSION_ID --cwd /path/to/project
```

## 自动邮件通知

- Rust 回归 42 项通过；邮件专项 11 项通过，覆盖中文会话名、Goal 首次完成标注、旧 Goal 恢复、事件去重、子 agent/中断过滤、配置隔离、原有通知保留、认证失败记录、重试及 TLS 安全边界。
- 本机 TLS SMTP 集成验证了异步投递、慢 SMTP 不阻塞通知回调、重复事件不重复发信。
- 原版 Codex 0.158.0 实测：一次 `exec` 完成自动投递邮件，随后通过 PTY wrapper 恢复同一测试会话并完成另一轮，再自动投递第二封。两封邮件均含真实 Session ID 和本地会话名，测试没有要求模型调用发信工具。
- 换用有效客户端授权码后，网易 SMTP 认证及测试邮件投递通过，服务器已接受邮件；本机自动通知已启用。收件箱是否实际收到尚未人工确认。

复现方式和配置见[邮件通知](docs/mail.md)。

## 实验性 tmux 接入

同日本机 Codex 0.158.0、tmux 3.2a、WSL2：

- `reptyr` 普通模式及 `-T` 在独立测试 PTY 上均返回 `Operation not permitted`；系统 `ptrace_scope=1` 未修改，失败未结束目标进程。因此未加入普通终端进程迁移。
- 新增 `codex24h attach <PID>`，仅连接已有 tmux 单窗格窗口。43 项 Rust 测试通过；已安装 release 的 4 项 attach 集成测试和 10 项原有 PTY 回归通过。
- attach 集成覆盖滚轮冻结、后台输出、原生风格选项、尺寸变化、断开再接入保持 PID、外部终止信号只断开客户端、源会话被删除后保留任务，以及拒绝普通 PTY 时不影响目标。
- 原版 Codex 实测：先提交 `sleep 12` 与算术回答任务，执行中接入；接入后完成，PID 和启动时间保持一致。随后历史输入召回、Tab 补全、模型菜单及断开后原 tmux 终端继续输入均通过。
- 测试使用独立本机 tmux server 和现有 Codex 登录，未接管用户正在工作的窗格。未修改全局 tmux、termInfo 或系统权限配置。原生任务测试关闭了测试会话的邮件通知。

接入前历史导入、普通终端迁移、多窗格布局、实体手机操作仍不在支持范围内。命令与边界见[接入说明](docs/attach.md)。

### 扩展交互与并发验证

同日本机扩大验证范围后，修复两项清理问题：多个 attach 顺序退出留下临时会话；源会话已删除时，两个 attach 同时退出可能删除最后的窗口引用、结束目标进程。两者均先在独立测试进程上复现，修复后回归通过。清理现在依据实际窗口引用，并对本用户的清理操作加锁。

- 最终 Rust 单测 43 项通过；**已安装 release** 的 attach 集成 13 项、原有 PTY 回归 10 项通过。
- 连续 8 次接入、断开，每次约 0.49–0.50 秒显示目标画面，没有残留临时会话或客户端。此数字仅是本机测量，不是性能保证。
- 两端同时接入时可分别冻结；原会话当前窗口不变。源会话删除后的并发断开连续验证 4 轮，目标 PID 存活且始终保留一个承载会话。
- 搜索、OSC52 复制与文本导出一致；本地操作未泄漏到目标输入。中文、emoji、分段粘贴和包含断开快捷键的粘贴均正确处理。
- 40 列窄屏、反复 resize、模拟 SSH 终端挂断、目标退出期间继续浏览、8 MiB 高速输出均通过。输出压力阶段不读取外层终端，目标仍能完成输出，冻结正文保持不变。
- 真实 Codex 验证了任务执行中接入、PID/启动时间不变、草稿保留、历史输入、Tab 补全、接入已打开的模型菜单，以及断开后原终端继续操作。

**真实历史滚动有明确模式差异，不能把所有原生验证概括为“滚动通过”：**

| 原进程模式 | 100 行真实回答的结果 |
|---|---|
| 默认全屏 | 画面冻结成功，但 tmux `alternate=1 history=0`，滚轮只有 1 个画面，聊天历史滚动要求未通过。原始终端输出为单元格重绘，没有换行滚动；已在列表、状态栏和文档提示此限制。 |
| `codex --no-alt-screen` | `alternate=0 history=111`；输出期间保持冻结，25 次滚轮操作得到 25 个不同历史画面，下滚到底部恢复跟随。 |

两种模式均使用独立本机测试会话和现有登录，没有重启或接管用户工作中的 Codex。默认全屏的聊天历史重建仍未实现；本次修复也未改变这个边界。实体手机触控和真实审批执行不在本轮实机覆盖范围内。


## 远程组合键透传修复

本机 tmux 3.2a、Codex 0.158.0，沿用现有登录。

- 修复前，attach 的普通 tmux 客户端会丢弃 CSI-u 和 modifyOtherKeys 编码的 Shift+Enter；经典 Shift+Left 在默认配置下可传递，但会被 tmux root 自定义绑定截获。
- 修复后，安装版 release 的 **15 项 attach 集成、11 项普通 PTY 回归全部通过**；Rust 单测 43 项通过。传统 Shift/Ctrl 方向键、Shift+Tab、增强 Shift+Enter、Alt+Enter、带修饰键的翻页键均逐字节一致，包括模拟 SSH 分段到达。
- 在 `extended-keys off` 且故意绑定 tmux `S-Left` 的情况下，按键仍原样送到目标，原 tmux 设置和绑定没有被改动。大段中文、emoji、引号、换行及包含断开前缀的粘贴保持完整。
- 已安装版本重复接入 8 次，每次约 0.49 秒显示目标；8 MiB 输出压力测试约 0.69 秒，冻结视图保持不变。正常启动 2 MiB 输出测试约 0.04 秒，wrapper RSS 4352 KiB。数据仅代表本机本次测量。
- 已安装 release 的 `tests/native_keys.py` 和 `tests/native_keys.py --attach` 均实际发起两道原生问题，左右切换后逐题选择“香蕉”和“蓝色”、按 Enter 提交，模型返回两项实际选择。测试使用已有认证并关闭邮件。测试等待启动稳定后再切换 Plan 模式，避免启动期间模式被重置；会产生真实模型请求。
- 原生多问题菜单标注使用裸左右方向键切换问题；实测 Shift+Left 在该菜单没有切换动作，不能将原样透传等同于 Codex 自带该键绑定。

上述验证使用本机 PTY 和 tmux，没有覆盖用户具体 SSH 客户端、实体手机键盘或物理按键。客户端在本地截获的快捷键不会到达 wrapper。


## 实际 TUI 场景扩展（2026-09-28）

在本机 Codex 0.158.0 上扩展 `tests/native_keys.py --extended`，普通启动和 `--attach` 各执行连续多轮真实模型交互。仅使用独立测试 tmux server、现有认证；关闭测试邮件，不碰用户正在工作的窗格。测试问题没有实际执行任务或改文件的授权。

| 场景 | 本轮观察 |
|---|---|
| 两道问题左右切换，逐题选择 | 香蕉、蓝色均作为实际答案提交 |
| 选项附加中文和 emoji 备注 | Tab 进入备注，Shift+Tab 后提交，原生结果保留“周五交付” |
| 等待回答时搜索历史 | wrapper 本地搜索结束后仍停在第二题，没有把搜索词当作回答 |
| 等待回答时断开、重新 attach | 目标 PID 存活，仍能继续回答第二题 |
| 100 列→42 列→100 列 | 窄屏菜单能选择，恢复宽屏后正常提交 |
| 粘贴结束序列后紧接 Enter | 实际算术问题得到 43，没有出现 Enter 卡死 |
| 下一轮输入历史与 slash 补全 | Up 召回、Down 恢复草稿、`/mo` + Tab 完成 `/model` |
| Escape 中断选项问题，再发一次 | 第二次仍能选择和提交 |
| 真实 80 行回答期间冻结 | 正文不随后台输出变化，回到底部能查看结果 |
| 真实问题无人操作 135 秒 | 问题仍等待回答，之后可正常选择；本次未复现社区报告的约 120 秒空答案自动提交 |

发现并修复两个 wrapper 缺陷，均先获得失败再验证修复：

1. 运行中的 PTY 上报 `0×0`、`1×80` 或 `24×1` 时，旧版因尺寸校验失败退出。现保留上一有效尺寸，正常尺寸恢复后继续输入；普通启动与 attach 均覆盖。这里只模拟了终端尺寸事件，未证明某款手机客户端会发送这些事件。
2. 冻结画面从宽屏缩小后再放大，旧版永久丢失被裁掉的字符和行。现保留完整快照，仅在显示时裁剪；新增缩小、后台覆盖、恢复尺寸的单测和 PTY 回归。此项不等同于自动重新排版整段历史。

最终安装版 release：43 项 Rust 单测、12 项普通 PTY 回归、15 项 attach 集成通过；普通启动和 attach 的完整扩展原生场景也通过。真实冻结缩放对比中，attach 外层输出改用 tmux 的 Unicode VT 解析器回放核对，避免 ASCII 简化测试解码器在中文覆盖旧字符时留下假残影。另将问题出现条件收紧为真实 `Question 1/2` 菜单，避免启动提示里的 “Enter” 误触发测试步骤。

原生场景测试是可选的，会产生模型请求；模型输出格式或界面文案变化可能需要调整断言。固定测试问题只验证交互链路，不代表审批、所有 MCP 工具、系统剪贴板、IME 组字和所有真实 SSH 客户端均已覆盖。

```bash
CODEX24H_TEST_BIN="$HOME/.local/bin/codex24h" python3 tests/native_keys.py --extended
CODEX24H_TEST_BIN="$HOME/.local/bin/codex24h" python3 tests/native_keys.py --attach --extended
CODEX24H_TEST_BIN="$HOME/.local/bin/codex24h" python3 tests/native_keys.py --attach --idle-question
```

### GitHub 线索与边界

以下来自 OpenAI Codex 仓库的用户报告，查阅日期为 2026-09-28。报告中的旧版本、终端组合不等于本机已复现。

- [#24235：Termius 手机滚动历史混入旧画面](https://github.com/openai/codex/issues/24235)。支持继续关注手机鼠标事件与冻结显示；本机 PTY 测试不能替代实体 Termius 手势测试。
- [#28167：tmux 粘贴后立即 Enter 卡住](https://github.com/openai/codex/issues/28167)、[#12645：tmux 任务完成后 Enter 失效](https://github.com/openai/codex/issues/12645)。本轮直接启动和 attach 连续交互没有复现，但只覆盖本机版本与注入的终端字节。
- [#5259：窄屏历史在放宽后不能重新排版](https://github.com/openai/codex/issues/5259)，该 issue 已关闭。不要把本轮“快照不再丢字”解释为历史语义重排；wrapper 无法从业务文本之外恢复所有被 Codex 硬换行的段落。
- [#12882：输入框缺少 Shift+方向键选区](https://github.com/openai/codex/issues/12882)。这是原生编辑能力诉求，与 wrapper 吞键不同；此前本机测试也没有得到 Shift+Left 选区。
- [#34455：请求配置问题自动超时行为](https://github.com/openai/codex/issues/34455)。本轮真实问题等待 135 秒仍可回答；不据此保证所有版本、模式、设置永不超时。

已知仍未解决：attach 默认全屏 Codex 的历史重建、接入前完整历史 hydration，以及客户端把滚轮转换成方向键或在本地截获快捷键的情况。保持原生 UI 的纯 PTY wrapper 不能可靠猜测这些按键原本代表什么。

## 浏览时保留输入区（2026-09-29）

安装版 release：46 项 Rust 单测、12 项普通 PTY 回归、15 项 attach 集成通过。`tests/native_features.py` 与 `--attach` 均使用本机 Codex 0.158.0 和已有登录，真实生成 40 行回答后翻页：上方历史保持不变，下方能继续输入中文草稿，`/mo` + Tab 完成原生 `/model`，回到底部恢复完整原生画面。测试关闭邮件，使用独立本机 tmux server。

默认实时区域为 6 行，按原生光标位置或屏幕底部截取，不识别业务文本。因此长输入、补全列表和提问菜单可能显示不全，可调整高度或回到底部；不宣称能自动识别完整输入框。单测覆盖实时区域的鼠标坐标映射。未通过实体 Xshell / Termius 客户端验证物理滚轮、IME 和图片剪贴板。

Windows 经 SSH 零配置粘贴图片尚未实现；本次没有保留客户端 PowerShell 脚本或图片转发助手。

## 公网 SSH 回连与 Windows 客户端探测（2026-09-29）

使用用户提供的 SSH 入口回连当前 WSL，仅创建独立测试会话，沿用已有 Codex 登录，关闭通知邮件。连接地址、账户凭据不写入测试代码或仓库。

- Windows `System.Drawing` 生成 96×64 绿色 PNG，通过 OpenSSH `scp`（SFTP）上传到远端临时目录，SHA-256 一致。实际 SSH PTY 中粘贴远端路径，codex24h 内的原生 Codex 显示 `[Image #1]`；提交后模型正确回答绿色。没有读取或改动 Windows 剪贴板，上传由测试程序完成，不代表已实现自动图片粘贴。
- 同一 SSH 会话再生成 40 行回答：翻页后历史冻结，下方中文草稿实时显示，`/mo` + Tab 原生补全通过；回到底部正常。
- 首次 SSH 启动没有继承当前终端的代理环境，原生 Codex 报 `account/read failed during TUI bootstrap: ... workspace routing discovery timed out` 并退出。仅为测试会话补齐相同代理环境后通过，未修改全局 SSH、代理或 Codex 配置。这是本次启动失败原因，不能据此归因此前所有卡顿。
- 本机实际安装的 Xshell 为 `E:\xshell\Xshell.exe`，安装记录版本 8.0.0095。新启动窗口被“要继续使用此程序，您必须应用最新的更新或使用新版本”阻止进入终端，故 **Xshell 图片粘贴、滚轮和物理快捷键仍未实测**。弹窗截图保存在忽略目录 `test-artifacts/xshell-update-required.png`；未升级或修改现有客户端，已关闭本次启动的进程。
- 用户确认本机没有 Termius，本轮不测试它。SSH 测试使用 OpenSSH + tmux 终端回放，不能作为 Xshell / Termius 已验证的证据。

测试结束关闭专用 SSH control master 和测试 tmux server，删除远端测试图片与临时网络环境文件。同机 WSL 直接读取宿主 Windows 剪贴板不构成 SSH 图片转发，本轮没有采用该捷径。

### 更新后的 Xshell 实机粘贴（2026-09-29）

用户更新后，Windows 安装记录为 Xshell **8.0.0110**。新建独立 Xshell 窗口，经用户提供的 SSH 入口连接当前 WSL；原有窗口保持不动。服务端运行 raw-mode 接收程序，启用 bracketed paste 并记录实际收到的字节，避免与模型响应或 wrapper 行为混淆。

| 剪贴板内容与操作 | 实际结果 |
|---|---|
| 测试文字，Ctrl+Shift+V | 收到完整文字与 bracketed-paste 边界，共 33 字节 |
| 生成的位图，Ctrl+Shift+V | 未收到图片或路径 |
| 同一位图，Shift+Insert | 未收到图片或路径 |
| 再放入测试文字，Ctrl+Shift+V | 收到完整文字与 bracketed-paste 边界，共 33 字节 |
| 复制 PNG 文件（FileDropList），Ctrl+Shift+V | 未收到文件路径，也没有出现上传界面 |

整个接收流严格等于前后两段文字对照，没有额外输入。测试时 Windows 确认剪贴板包含图片；图片来自生成的 PNG，不使用用户剪贴板内容。原剪贴板暂存在测试进程内并恢复，测试窗口关闭。Windows 与 WSL 的墙上时钟存在偏差，因此结果按完整字节流核对，不使用跨系统毫秒时间划分断言。

结论：**本机这版 Xshell 的上述默认粘贴方式不满足“自动上传剪贴板图片并插入远端路径”要求**。这不是 codex24h 吞掉图片；本轮直接在 SSH 接收端测试，尚未经过 wrapper。未测试手动 SFTP、拖拽文件或客户端脚本，不将它们等同于零配置图片粘贴。结果保存在忽略目录 `test-artifacts/xshell-paste-results.json` 和 `test-artifacts/xshell-received.jsonl`。

### Tabby 图片插件实测（2026-09-29）

Windows Tabby **1.0.237**，安装 npm 发布的 `tabby-ssh-image-clipboard` **0.1.0** 到用户插件目录。使用同一已安装客户端的独立本机测试配置，建立真实 SSH 连接；通过 Windows 系统剪贴板放入生成的 96×64 绿色位图，自动化发送 Ctrl+Shift+V。服务器上无需安装图片传输助手。

原版插件首次 SSH 连接时漏掉了 split tab 内的活动连接：快捷键已触发，剪贴板确实包含图片，但插件 activeContext 为空，因此没有上传。为本机安装加入一处兼容修复：`initializePasteHook` 中调用 `pasteImage()` 前，先调用 `checkAndSetActiveSession(this.app.activeTab)`。修复仅位于 Windows 用户插件目录，不是 codex24h 的代码或随仓库发布的功能；插件重新安装或升级可能覆盖它。

修复后重新加载插件并建立全新连接，确认粘贴前 activeContext 为空；第一次 Ctrl+Shift+V 即识别当前 SSH 会话，经 SFTP 写入 `/tmp/clipboard_<timestamp>.png` 并插入带引号的远端路径。接收端确实收到路径，上传文件解码为 96×64、RGB (50,205,50) 的测试位图。随后在该连接的 codex24h 中再次粘贴，原生 Codex 显示 `[Image #1]`，提交只问颜色的提示后模型回答“绿色”。沿用现有 Codex 登录，关闭测试邮件；未让 agent 执行上传工具。

证据保存在忽略目录 `test-artifacts/tabby-received.jsonl`、`test-artifacts/tabby-patched-native-image-success.png`。自动化通过实际 Electron 窗口的键盘事件和系统剪贴板测试，不是物理键盘人工操作。测试期间短暂保存并恢复剪贴板；若检测到其他操作更新了剪贴板则保留较新的内容。测试 SSH、独立客户端进程和本机调试端口均关闭，原有 Tabby 窗口保留；另打开正常配置的新窗口加载已安装插件。


### 2026-09-29：会话占用和鼠标拖选

- Codex 0.158.0 原生独立进程：两个 wrapper 恢复同一测试 session，复现 `This conversation is open in another app`；旧进程 Ctrl+C 退出后，第二个按 R 成功继续。显式 session 接管成功终止旧写入者并恢复同一 UUID，未操作用户工作会话。
- `python3 tests/e2e.py`：12 项通过，包含退出、信号清理、原生按键透传和冻结浏览。
- `cargo test --lib`：51 项通过，新增直接拖选、后台重绘期间保留按下时文本、固定输入区历史选择。
- `/usr/bin/python3 tests/session.py`：7 项真实内核锁/进程测试通过，包括拒绝共享 app-server、多会话和错误目标；不删除原生锁文件。
- Xshell 实际 Shift+Left 尚未完成本轮验证：Windows 前台窗口返回 0，未向工作窗口发送测试键。用户反馈 Tabby 中对应快捷键可用。

- `python3 tests/attach.py`：15 项通过，包括修改键透传、重复接入清理和原任务存活。
- Tabby 原生 SSH 上，插件 Ctrl+Shift+V / Ctrl+V 均上传 PNG，并在 codex24h 的 Codex 0.158.0 输入框显示两个图片附件；修复普通字符路径插入不稳定的问题。安装名统一为 `tabby-ssh-image-paste`，本机 0.1.2，原插件已备份到插件扫描目录以外。

- 真实 Codex `request_user_input`：三项中文选项及换行描述，隐藏光标，固定区依次显示当前第 1/2/3 项；按方向键时历史上部保持不动。增大最大高度后显示问题和完整选项区，不包含前面的任务输入。

- 插件 0.1.3：Windows 双击安装器已实际运行、重复安装成功，文件与目录联接均验证；升级保留其他插件并备份旧文件。更新测试覆盖校验失败、下载失败、成功替换、禁止降级及临时文件清理。

- 插件 0.1.4 改用 Chromium 网络栈，修复本机 Node HTTPS 的域名解析失败。独立真实 Windows Tabby 窗口的启动检查成功下载安装 GitHub 当时缓存的 0.1.3；随后以 v0.1.4 tag 清单调用同一更新函数，实际替换 JS / LICENSE / package.json，三份 SHA-256 均与发布清单一致。未重启用户工作窗口。

### 2026-09-29：整段图文与手动更新

- Windows Tabby 原生 SSH → codex24h → Codex 0.158.0：剪贴板仅提供 HTML 与文字，没有原生位图；一次 Ctrl+Shift+V 按顺序显示 before-one、Image #1、between-two、Image #2、after-three。图片分别来自内嵌 PNG 和本地 file URL，使用真实 SFTP 上传，未向模型发送测试任务。
- 真实 Electron DOM / 图片解码测试覆盖 HTML 顺序、段落、选区 fragment、忽略脚本、Word 的 v:imagedata、本地/内嵌图片及公开 HTTPS 图片读取。粘贴服务测试覆盖多图顺序、剪贴板快照、唯一文件名和第二张图失败时不插入部分内容。
- 插件 0.1.5 删除启动自动更新；独立 install.cmd 的 Windows 实测覆盖脚本位于含空格路径、GitHub 下载、解压与安装。后续升级仍由用户双击同一脚本触发。

- Ctrl+] + 修复：52 项 Rust 单元测试通过；真实 Tabby 键盘事件先进入 COPY，再发送 Ctrl+]、Shift+=，成功回到浏览并显示 Live input max: 8 rows，保留历史。
- 固定 install.cmd 在本机实际从 GitHub 下载 v0.1.5，先升级独立测试配置，再安装到用户插件目录；无后台自动更新代码。

### 2026-09-29：远程交互审查

- 复现鼠标拖选后持久停留 COPY：后台 PTY 继续输出但画面冻结，Ctrl+C 只退出本地模式。修复为松开即复制并恢复原状态；旧鼠标协议的 button-3 release、窗口外释放后的下一次输入均覆盖。持续原位 WORK 计时测试连续拖选四次仍递增，无需 Ctrl+C；冻结历史的回归仍通过。
- 删除服务器文件导出、e 键和 CODEX24H_EXPORT_DIR；复制测试改为检查 OSC52，而非读取服务器文件。空白选区保留原客户端剪贴板。
- 实机 Tabby 1.0.237 的右键 Export to file 属于客户端，原本写入 Windows 本地。插件 0.1.6 已在真实 SSH 标签页过滤此项，保留用户原有“右键短按粘贴、长按菜单”配置。独立窗口右键菜单粘贴 HTML 图文后，真实 Codex 显示两张附件、前中后文字各一次；测试图上传和测试窗口已清理，未操作用户工作会话。
- 插件测试覆盖原生菜单/自定义快捷键共用 paste 入口、纯文字仅粘贴一次、非 SSH 标签不修改、装饰器解除，以及失败粘贴清理本次临时文件。

本轮验证：54 项 Rust 单元测试通过；PTY 端到端与 attach 回归分别覆盖 13 / 15 项，修改后的复制与持续刷新用例再次通过；插件两组服务/菜单测试及真实 Tabby 菜单图文粘贴通过。
