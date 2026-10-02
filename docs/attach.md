# 在 tmux 中重连到正常 codex24h 前端

按 tmux 前缀键（默认 **Ctrl+B**），松开后按 **h**。弹窗列出共享后台中的主会话，输入编号选择一次，随后进入正常 codex24h：直接接收 Codex 的终端输出，支持历史滚动、固定实时输入区和搜索。

原页面和后台任务保留。原页面的未发送草稿不会复制到新前端；关闭新前端后，仍可回到原页面继续编辑。按 **Ctrl+]，然后 d** 直接返回；Codex 正常退出也会关闭弹窗，不再将后面的 shell 留在包装器里。运行中的任务需要返回原页面时，使用这个返回快捷键；Codex 自己的中断命令仍保留原生含义。

## 配置快捷键

```bash
mkdir -p ~/.config/codex24h
codex24h attach --tmux-config > ~/.config/codex24h/tmux.conf
tmux source-file ~/.config/codex24h/tmux.conf
```

在实际使用的 tmux 配置文件（通常为 `~/.tmux.conf`）加入：

```tmux
source-file ~/.config/codex24h/tmux.conf
```

此配置绑定前缀表中的 `h`；已有自定义绑定时可换成空闲按键。重复加载替换同一绑定。快捷键不向原 Codex 输入框写入任何命令，不调用模型。需要支持 `display-popup` 的 tmux（已测 3.2a）；支持分屏，按触发时的客户端和窗格定位。重复触发不会叠加弹窗，客户端断开会清理新前端。

## 会话选择和范围

列表优先显示原页面工作目录中的会话；`a` 显示全部，`n` / `p` 翻页，`q` 取消。显示会话名、完整 UUID 和目录，不按“最近使用”猜测当前会话。原生 Codex 未提供可靠的终端到当前会话 ID 映射，因此每次打开新前端需要选择一次。

列表仅包含本机共享后台实际持有写入锁、可确认是主会话的记录。独立模式、显式 `--no-daemon`、远程连接、已经退出到 shell 的页面不支持此入口。不会终止原进程、释放会话锁或重启后台服务。

新前端使用原页面正在运行的 Codex 可执行文件和 `CODEX_HOME`，避免 PATH 已更新时串用另一个版本；原页面升级重开后，快捷键也跟随新版本。没有加入会强制独立后台的 `-c` 覆盖，也不使用隐藏 agents 入口的远程连接方式。

**Ctrl+]，然后 r** 的列表绑定这次明确选择的主会话，标题显示其 UUID；共享模式从只读历史数据库读取请求标签，正文仍由原生终端显示。原生 `← for agents` 可以照常切换；请求列表不自动跟随原生 `/new`、`/resume` 或 agents 切换，定位主会话请求前需返回该主会话。匿名共享会话的自动识别不在此入口范围内。

旧的终端镜像实现及 `codex24h attach <PID>`、`--list` 入口已移除。它只能镜像默认全屏重绘，无法获得正常包装器的滚动历史，且 Codex 退出后仍会包裹 shell。新的快捷键不再使用临时 tmux 分组会话、控制连接或私有 terminfo，也不再依赖 `tic`。

## 验证

```bash
cargo build --locked
python3 tests/attach.py
# 独立真实测试：提交一个 sleep 任务，验证重连、后台连续运行和正常退出
python3 tests/native_reconnect.py
```

真实测试使用独立 tmux server 和测试会话，邮件关闭。旧版本可以通过 `--codex /path/to/codex` 指定；`--session <测试会话 UUID>` 只重连已有测试会话，不提交新任务。当前验证组合见 [TESTING.md](../TESTING.md)。
