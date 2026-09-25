Commit: 832f94ebca7792accf3a918f29d42cc130fb075b

# 关闭 X 改为居中确认弹窗（一次点击不再直接退出）

## 背景（Context）

上一版是「双击确认」的闩锁：第一次点击在 `WindowUi` 上挂一个 5 秒的
`close_confirm` 时间戳，`confirm_armed` 判定第二次点击是否还算「新鲜」，
点别处或超时解除。问题在于这个中间态**完全不可见**：用户看不出第一次
点击已经进入确认态，第二次点击等于在盲点；而且「关闭 X 会退出整个应用」
这件事本身没有任何文案说明（点二次窗口的 X 等于退掉全部窗口）。本次改成
显式弹窗：一次点击弹出居中确认框，框上写清这次退出会带走几个窗口，点
Quit 才真的退出。

## 变更摘要（Change Summary）

- 新增 `crates/app/src/ui/close_dialog.rs`：`WindowUi.close_dialog` 为真时，
  在该窗口自己的渲染 pass 里 `show` 一个 `egui::Modal`
  （`Id::new("close_dialog").with(win_id)` —— 按窗口加盐，两个窗口不会共用
  同一个 modal id；backdrop 黑 96 alpha；`Frame::popup` + 12px 内边距）。
- 内容全部来自纯函数与既有 token：标题常量 `TITLE =
  "Quit terminator-rust?"`；副行由纯函数 `quit_hint(window_count)` 生成
  （<=1 个窗口用单数文案，否则报出窗口数）；`chrome::metrics(font_size)`
  缩放出的固定宽度（min 与 max 同为 330*m.s，只给 min 时按钮行会撑满
  Area 的首帧可用宽度而变成 600 宽）与两个 96x28*m.s 按钮；布局为
  `Layout::right_to_left`，故第一个按钮 Quit 贴右（用调色板红
  `pal.normal[1]`）、Cancel 在其左；两者都用 `Sense::CLICK`（非可聚焦，
  遵守全局「控件不抢键盘焦点」规则）。
- 退出路径集中在一处 `request_quit`：`send_viewport_cmd_to(ViewportId::ROOT,
  ViewportCommand::Close)` —— 与 Ctrl+Shift+Q 同一条路径，退掉**全部**
  窗口，而不是只关当前窗口。
- 取消路径：Cancel 按钮、backdrop 点击（modal 在最上层，所以**再点一次
  关闭 X 也落在 backdrop 上**）、Esc（`ModalResponse::should_close`；egui
  只对最上层 modal 消费 Esc，不会顺带触发别的快捷键）。三者都只清
  `close_dialog`，不动窗口与 pane。
- 模态性：`render/screen.rs` 新增 `modal` 门禁 —— egui 的 modal layer 只挡
  WIDGET 交互，挡不住项目自己的 raw pointer 路由与纯 rect 探针（分隔条、
  `pane_interact`、右键菜单），因此按帧显式屏蔽；`input/keyboard.rs` 在弹窗
  存在时 early return，按键、粘贴、剪贴板都到不了 pane。
- 关闭 X（`ui/tabs_widgets.rs::edge_cells`）现在只置
  `WindowUi.close_dialog = true`（弹窗存在期间该 cell 仍画红色激活底色）；
  旧的 `close_confirm` / `CLOSE_CONFIRM_SECS` / `confirm_armed` 全部删除。
- 挂载点：`main.rs`（根窗口 pass）与 `windows.rs`（每个二次窗口的
  `show_viewport_immediate` 回调）都在 inspector 之后调用 `show`，保证弹窗
  在最上层。`WindowUi` 是「瞬态、不持久化」状态，state.json 格式不变。

## 影响面（Impact Surface）

- 用户可见：关闭 X 一次点击 -> 居中确认框；Quit 退出整个应用（含二次
  窗口），Cancel / Esc / 点框外（含再点一次 X）取消，取消后所有 pane 原样
  保留，窗口不消失。
- 弹窗存在期间输入被完全挡住：不漏键进 pane、不触发粘贴/剪贴板快捷键、
  不会误点 pane、分隔条或窗口边角 resize。
- 尺寸：内容宽 min AND max 双钉到 `330*m.s`（+ 2x12 边框内边距）。只给
  `set_min_width` 不够 —— right_to_left 的按钮行会撑满首帧可用宽度
  （egui `Spacing::default_area_size` 600x400），弹窗会稳定在 600 宽
  （开发中实测过）。e2e 因此把弹窗断言为「一个居中实心框」（面积带 +
  填满 bbox >= 90% + 居中）而不是算术宽度：字体 15 下实测 bbox
  356x138（含 1-2px 抗锯齿外溢）。
- 键盘/IME/持久化格式均无影响；旧 state.json 里也不存在该字段。

## 验证（Verification）

- `scripts/bin/e2e-window-controls.sh` 的 R6 重写为 6 段并全绿（整套 R1-R6 /
  S1-S5 全绿，末行 `WINDOW-CONTROLS E2E: PASS`）：一次点击后应用与窗口仍
  存活，且弹窗是单个居中实心框（scrot 差分：`dialog: 49085px brighter
  (5.1% of the window), bbox 356x138 solid 99.9%, centre 600,400 vs window
  centre 600,400 [OK]`）；弹窗期间打字不进 pane（`ctl capture` 取回，配合
  弹窗前的负对照 —— 同一段打字在无弹窗时确实进 pane）；Esc、再点一次 X、
  Cancel 三种取消（均为 `dialog dismissed: 0px still brighter (limit 300)
  [OK]`）；Quit 用记录下来的 frame right/bottom 反推按钮中心
  （`Quit button centre 818,493`，frame right/bottom 878,519 —— 即
  right-60 / bottom-26，与 96x28 按钮 + 12px 内边距一致）并真的退掉应用。
- `cargo build -p app -p ctl --bins` 通过（e2e 脚本内置这一步）；
  `cargo fmt --check` 与 `cargo clippy --workspace --all-targets -- -D warnings`
  均 exit 0。
- `cargo test --workspace` 全绿（21 个测试目标；`-p app` 173 passed，内含
  `quit_hint` 单测）。顺带修掉一个既有测试 bug：
  `paths::tests::runtime_dir_lives_under_terminator_rust` 在不设
  `XDG_RUNTIME_DIR` 的环境里必失败（`runtime_dir()` 退回 `~/.terminator-rust`，
  而断言 `ends_with("terminator-rust")` 对 `.terminator-rust` 这个路径分量不
  成立）。改为断言纯函数 `runtime_dir_from` 的两个分支，环境无关，
  带/不带 `XDG_RUNTIME_DIR` 均通过。

## 测试覆盖（Test coverage）

| 功能点 | 测试 | 入口 |
| --- | --- | --- |
| 一次点击 X 只开弹窗，应用与窗口存活 | R6 第 1 段：scrot 差分（面积带 + 实心度 + 居中） | `scripts/bin/e2e-window-controls.sh:326-368` |
| 弹窗期间按键/粘贴不进 pane | R6 第 2 段：`ctl capture` + 弹窗前负对照 | `scripts/bin/e2e-window-controls.sh:371-379` |
| Esc 取消 | R6 第 3 段（取消后 mask 归零 + 应用存活） | `scripts/bin/e2e-window-controls.sh:381-399` |
| 再点一次 X 只取消（落在 backdrop） | R6 第 4 段 | `scripts/bin/e2e-window-controls.sh:401-414` |
| Cancel 按钮取消 | R6 第 5 段（按钮中心由实测 frame right/bottom 反推） | `scripts/bin/e2e-window-controls.sh:416-433` |
| Quit 退出整个应用 | R6 第 6 段（`wait_pid_gone`） | `scripts/bin/e2e-window-controls.sh:435-446` |
| `quit_hint` 文案（1 个窗口 vs N 个窗口） | 单元测试 `quit_hint_single_window` / `quit_hint_counts_windows` | `crates/app/src/ui/close_dialog.rs:105-116` |
| chrome 其它行为不回归（R1-R5 / S1-S5） | 同一套 e2e 全绿 | `scripts/bin/e2e-window-controls.sh` |
| 多窗口不回归（W1-W10，含次级窗口） | `scripts/bin/e2e-windows.sh` | — |

## 顺带（Also In This Commit）

- `scripts/bin/deploy-remote.sh` 新增 `--keep-running`：只装不杀（stage 5
  打印 `DEPLOY-KEEP:<pids>` 后直接退出，`rm` 陈旧 socket 也跳过），用于目标机
  窗口里有活会话时发布；`pids_of` 去掉 readlink 尾部的 ` (deleted)`（repack
  原地替换目录）。
- `crates/paths/src/lib.rs`：`runtime_dir_lives_under_terminator_rust` 改成
  断言纯函数两分支，不再随 `XDG_RUNTIME_DIR` 有无而红（详见验证段）。
- 同名时间戳的 [rename-editor-ime-interrupt.md](./rename-editor-ime-interrupt.md)
  是同一提交里的 IME 修复。

## 相关文档（Related Docs）

- [agents.md](../../../agents.md) —— chrome 条目：关闭 X 弹窗、模态门禁、
  min/max 宽度钉定与 356x138 实测
- [agents/e2e-suites.md](../../../agents/e2e-suites.md) —— R6 覆盖说明
- [agents/verified-end-to-end.md](../../../agents/verified-end-to-end.md)
- 同日变更：[rename-editor-ime-interrupt.md](./rename-editor-ime-interrupt.md)
