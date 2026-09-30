# Release 构建

用户安装预编译包，不运行此流程。当前支持 Linux x86_64、glibc 2.35+；构建使用 Ubuntu 22.04 和 Python 3.10，以免引入新系统 ABI。

本机构建需要 Rust、Python 3.10、pip 和 strip：

```bash
python3 -m pip install -r packaging/requirements.txt
bash packaging/build.sh
```

产物在 `target/release-assets/`：tar.gz 和对应 SHA-256。Rust 主程序和四个 helper 入口共享一个 PyInstaller onedir 运行时，不携带用户配置、授权码或源码仓库，不在每次启动时解压。

修改 Cargo.toml/Cargo.lock 中版本并更新文档后，提交到 main，推送同版本的 `vX.Y.Z` 标签。GitHub Actions 构建、测试并创建 Release，上传安装包和校验文件。开发构建可用 `cargo build --release`；源码辅助程序仍需 Python 3.11+，或 Python 3.10 加 tomli。

安装命令下载最新正式 Release。版本目录及原子 symlink 切换保证新启动的进程使用新版本；运行中的进程保留原版本，后续更新清理闲置版本。
