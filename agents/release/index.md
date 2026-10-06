Commit: 8ecb8ab47bc992a9522edf0394616e0007744e47

# 发布与回滚

- [deploy-remote.sh](../../scripts/bin/deploy-remote.sh)：Linux x86_64 构建、共同打包入口、SHA256 校验、远端 Python 3.8+ 安装器、桌面自启动、重启、IPC/进程实际路径检查。
- [release.py](../../scripts/deploy/release.py)：真实发布身份 preflight，SHA256 独立目录 `releases/<sha>`，校验包与已解压文件；先持久化不可覆盖 `journal/<sha>.json`，再原子切换 `current`。同包重试复用最初的回滚记录；旧请求不能覆盖后来版本。
- `--rollback <sha>` 恢复该包记录的上一目录并验证文件清单，再按部署流程启动和检查；首装没有上一版时拒绝，不移除当前版本。旧版本内容被改动也拒绝恢复。
- `--keep-running` 保留已运行实例和 socket；此时 smoke 只证明旧实例正常，新包在下次启动生效。部署前检查目标的 pane、shell 子进程和会话，不能因发布丢掉用户正在进行的工作。可用私有 HOME、XDG_RUNTIME_DIR 和 Xvfb 验证候选包。
- 重启仅处理 `/proc/PID/exe` 属于安装根目录的进程。进程候选包含完整路径与裸进程名；旧安装遗留的 ` (deleted)` 后缀仍需识别。`ctl list` 的 PID 是登录 shell，实际 SSH/TUI 是子孙进程；多实例用各自 socket 指定。
- [安装器测试](../../scripts/tests/deploy/test_release.py)：同版本号不同包、切换前后真实进程崩溃、重复安装/恢复、损坏内容、过时请求、路径穿越、首装无回滚目标，以及验收失败时不上传、不安装。
- [release.yml](../../.github/workflows/release.yml) 通过相对路径调用同一提交的 [ci.yml](../../.github/workflows/ci.yml)，全部成功后才构建发布包。Linux 包上传前调用 [verify-release.sh](../../scripts/deploy/verify-release.sh)，在私有目录安装并执行真实 IBus 输入法测试，之后复核 SHA256。macOS CI 检查不能替代原生 GUI 验收。
- 远端部署的构建阶段执行 fmt、Clippy、全仓 Rust 和部署测试；上传前执行相同的实际包验收。`--no-build` 仅跳过构建和重新打包，仍验收包；显式 `--rollback` 不依赖候选包检查，保留故障恢复入口。构建机需要 Linux IME 测试依赖。

构建包不会自动授权部署。生产发布的写能力、真实桌面启动和观察必须在授权的目标环境验证；本地隔离测试不能替代。
