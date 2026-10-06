Commit: 8ecb8ab47bc992a9522edf0394616e0007744e47

# 拼音行尾重叠与原生候选窗位置

精确填满一行时，终端光标仍停在末列，预编辑却应从下一行开始。X11 输入法仅收到一个坐标点，候选窗向上弹出时也可能遮住当前输入行。

- 根据终端 pending wrap、自动换行模式和首字宽度计算预编辑位置；末行使用临时滚动画面，取消后恢复，实际终端缓冲不变。
- X11 根据可见候选窗的实际尺寸避让输入行，仅在本应用拥有焦点且窗口属性匹配时处理；保留不中断组合、延迟回显跟随和重命名交接。
- 新增精确行尾、宽字符提前换行、末行取消和提交的真实 IBus 像素验收。
- 标签发布先运行同一提交的 CI；Linux 发布包及远端部署在上传前执行实际包输入法测试，验收失败停止后续安装。`--no-build` 同样检查包，回滚入口保持独立。

## 验证

408 项 Rust 测试、8 项部署测试、fmt、Clippy、macOS 全工作区全目标交叉检查、release 构建、actionlint 均通过。中文渲染与鼠标键盘回归通过。调试程序和实际发布包的完整 IME 测试通过，覆盖原生候选窗、延迟回显、行尾、取消、重命名和英文切换。

精确行尾复现中被盖住的已提交文字像素由 46 降至 0；M3e 三种边界分别保留全部 180、180、182 个字形像素。发布包在私有 Xvfb 中安装和恢复旧布局目录后均可启动，进程路径及 IPC/PTY 检查通过；未执行生产发布。

验收包 SHA256：`8f462e98d75931799b921e8131f30c34d158254e620c14fde8aefaf53ee5b13a`。

本机证据：`/tmp/terminator-closure-release-ime.log`、`/tmp/term-e2e-ime-MCK3Jj/`、`/tmp/terminator-closure-complete-rust.log`、`/tmp/terminator-closure-complete-deploy.log`、`/tmp/terminator-closure-final-artifact-smoke.log`。

## 验收边界

macOS 跳板可达，但测试机的两条已知 SSH 路径均不可达；原生候选窗实机验收仍是外部阻塞，不能宣称全平台可稳定发布。Fcitx 尚未完成与 IBus 同等级运行验收。Linux XIM 仍要求 libX11 ≥ 1.8.2；新增 `x11rb` 直接依赖使用锁文件已有版本，无新增运行时配置或持久化字段。

相关：[输入法](../../../agents/input-method/index.md)、[发布与回滚](../../../agents/release/index.md)、[测试索引](../../../agents/e2e-suites.md)。
