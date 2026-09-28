# 接入正在运行的 Codex（实验性）

如果 Codex 已经在 **tmux 的单窗格窗口** 中运行，可以在另一个终端或窗格执行：

```bash
codex24h attach --list
codex24h attach 12345
```

`--list` 显示当前 tmux server 中的窗格及起始进程 PID。该 PID 可能属于启动 Codex 的 shell，也可以用同一窗格内 Codex 自身的 PID。列表不表示每一项都是 Codex；请选择需要连接的窗格。不能从目标自身的终端接入。

默认连接 `$TMUX` 指向的 server，未设置时连接默认 server。自定义 socket 可指定：

```bash
codex24h attach --socket /path/to/tmux.sock --list
codex24h attach --socket /path/to/tmux.sock 12345
```

需要本机 tmux、Python 3 和 `tic`（通常由 `ncurses-bin` 提供）。不需要 root、重新登录 Codex或重新提交正在执行的任务。

## 连接与断开

- 原 Codex 进程、任务和输入状态继续使用。方向键、Tab、菜单与审批仍由原版程序处理；tmux 原有的键盘协议转换仍然存在。
- 滚轮和翻页由 codex24h 处理。上翻后冻结，返回底部后跟随；搜索和复制沿用原来的快捷键。
- **Ctrl+]，然后 d** 断开 wrapper，原任务继续运行。原终端也仍可使用；两端的输入会作用于同一个 Codex。
- Ctrl+C 仍发送给 Codex，可能中断任务；需要仅断开时使用上述快捷键。
- 原版 Codex 自己退出后，如果窗格返回 shell，wrapper 也会显示该 shell；它连接的是终端，不会解析文本判断任务生命周期。

只积累接入后 tmux 实际绘制到该客户端的内容，不导入接入前的历史，也不保证保留高频输出的每个中间画面。原生 Codex 若使用 alternate screen，或者窗口一直只做原位覆盖，能积累的历史可能有限。

tmux 管理共享窗口的尺寸；多个客户端尺寸不同时，遵循既有的 `window-size` 等设置，不保证每一端都能同时铺满。实验版拒绝多窗格窗口和正在 tmux copy mode 中的窗格，避免接入时改变分屏或把按键交给错误的模式。

邮件通知不会补注入既有 Codex：原来已配置的通知继续生效，原来没有的不会因为 attach 自动启用。

## 实现与边界

创建一个临时 tmux 分组会话，共享目标已有窗口，通过 PTY 承载 tmux 客户端，再接入现有 VT 解析和绘制循环。不启动第二个 Codex，不读取业务文本、不改 Codex 配置，也不复制登录凭据。

临时会话关闭自己的状态条和前缀键；源会话的设置不变。独立临时 terminfo 禁用此 tmux 客户端的 alternate screen，使 wrapper 可以积累滚动历史；不修改全局 terminfo 或 tmux 配置。正常断开会清理临时会话；若源会话已被删除，则保留连接会话，避免删除窗口的最后一个引用而结束任务。

普通终端进程的直接迁移尚不支持。本机 WSL2、`ptrace_scope=1` 下，`reptyr` 普通模式和 `-T` 均实测返回 `Operation not permitted`。此外，直接迁移还需要解决子进程、终端状态重建及断开后的 PTY 生命周期。因此本入口不会偷偷修改系统 ptrace 设置或强行迁移进程。

验证命令：

```bash
cargo build --locked
python3 tests/attach.py
# 原版 Codex：发送一个 sleep 任务，在任务运行中接入，再验证原生输入与断开
python3 tests/attach.py --native
```
