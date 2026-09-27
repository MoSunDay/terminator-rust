Commit: 95657c6a927aad2ece9d132a3923993ab904a294

# Tab 关闭需要确认

## 变更摘要

- 点击 tab 芯片的 X 或中键关闭 tab 时，先显示“Close tab?”弹窗；确认后结束该 tab 内的终端并关闭 tab，取消、Esc 或点击弹窗外则保留。
- 待确认目标用 tab 的 anchor pane id 标识。tab 换序后仍关闭原 tab；目标已消失时撤销待确认请求。
- 终端标题栏的 X 继续直接关闭对应 pane。窗口右上角的 X 继续弹出退出全部窗口的确认框。

## 影响范围

- 复用每个窗口的确认弹窗及其输入遮罩；关闭状态为瞬态，不修改持久化格式。
- `cargo test -p app`：182 项通过，包含 tab 换序与消失的目标解析测试。
- `scripts/bin/e2e-tab-close.sh` 在隔离的 Xvfb + openbox 会话中复现真实
  点击，核对 tab 标题、存活 pane 数量、确认／取消、次级窗口隔离与最后一个
  tab 的退出；CI 的窗口控制 job 运行此脚本，并安装既有图形脚本所需的
  `scrot`、`python3-pil`。
