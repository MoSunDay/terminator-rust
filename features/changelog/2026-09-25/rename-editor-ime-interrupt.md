Commit: 832f94ebca7792accf3a918f29d42cc130fb075b

# 改名编辑器不再打断 XIM 输入法上下文（中文组合输入从第一个键起生效）

## 背景（Context）

双击 tab 芯片或面板标题栏改名时，拼音会落成裸拉丁：得到 `小` + `hanzi`
而不是 `小汉字`；编辑器关闭后，该面板的第一个键同样落成裸拉丁。根因不在
本项目的事件处理，而在 egui/winit 两层叠加的机制：

1. `Response::request_focus()` 会调 `Memory::interrupt_ime()`，随后
   `ContextImpl::end_pass` 把该标记写进
   `PlatformOutput.ime.should_interrupt_composition`；
2. egui-winit 对 interrupt 的响应是 `set_ime_allowed(false)` 紧接
   `set_ime_allowed(true)`；
3. winit X11 把这对调用实现为 `remove_context` + `create_context`，即
   **销毁并重建** XIM 输入上下文（XIC）；而新建的 XIC 不会再被
   `XSetICFocus`（winit 只在窗口 `FocusIn` 时聚焦 IC）。此后每个按键都
   绕过输入法，直接以裸字节进控件。

编辑器在存活的每一帧都重新 request_focus，于是 interrupt 连续发生；并且
「从陈旧状态拿到焦点的那一帧」和「回车交出焦点的那一帧」也会触发同一个
开关 —— 这正是编辑器关闭后面板第一个键落成裸拉丁的原因。

## 变更摘要（Change Summary）

- 新增 `ui::focus_rename_editor(ctx, id)`（crates/app/src/ui/mod.rs）：用
  egui 的 Tab 导航路径交键盘 —— `surrender_focus` +
  `move_focus(FocusDirection::Next)`，聚焦当帧第一个 focus-interested
  控件，即紧随其后添加的编辑器；这条路径不碰 `interrupt_ime`。
- 两个编辑器都改为稳定 id（`tab_rename_edit` / `pane_rename_edit` 按 tab
  锚点 / pane id 加盐）并调用该辅助函数，`request_focus()` 全部删除。
  chrome 中唯一 focus-interested 的控件就是改名编辑器，故焦点不可能落到
  别处。
- `input/ime.rs::sync` 的让位条件改为「是否已经有人写过 IME 输出」
  （`ctx.output(|o| o.ime.is_some())`），不再用
  `ctx.egui_wants_keyboard_input()`：编辑器获得焦点 / 回车这两帧自身不发布
  输出，此时保留面板的 `Terminal` 输出，egui-winit 就不会看到 `Some` ->
  `None` -> `Some` 的翻转（该翻转同样会重建 IC）。

## 影响面（Impact Surface）

- tab 芯片与面板标题栏两处改名都能从**第一个键**起正常组合中文；编辑器关闭
  后面板输入法仍然活着（下一个键继续组合，而不是裸拉丁）。
- 面板 preedit 清理逻辑不变：仍由 `input/keyboard.rs` 的文本框 early return
  清 `WindowUi.ime`。
- 无编辑器的面板、Settings 面板输入框、非 X11 平台行为不变（Wayland/macOS
  的 winit 重新聚焦方式不同，但 interrupt 对它们同样是多余的 IC 抖动）。
- 回归覆盖：`scripts/bin/e2e-ime.sh` M4（tab 改名，第一个键即组合）、M5
  （面板标题改名 + 编辑器关闭后的面板第一个键）；原 M4 退出阶段顺延为 M7，
  M6（裸 Shift_L 切 libpinyin 英文直通）排到最后，保证组合类阶段停在中文
  模式。把四个源文件回退即可复现原始形态（`shellhanzi `）。

## 验证（Verification）

- `scripts/bin/e2e-ime.sh` 全绿（M1-M7），干净重建的普通构建上复跑一次仍
  全绿；负对照（回退四个源文件）在 M4 以 `shellhanzi ` 失败，证明该阶段确实
  咬住此 bug。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets
  -D warnings`、`cargo test --workspace`（0 failed）、
  `cargo build -p app -p ctl --bins` 全部通过。

## 相关文档（Related Docs）

- [agents.md](../../../agents.md) —— IME 硬知识条目 + M1-M7 e2e 条目
- [agents/e2e-suites.md](../../../agents/e2e-suites.md) —— 各 e2e 脚本覆盖
- [agents/verified-end-to-end.md](../../../agents/verified-end-to-end.md)
