Commit: 5f77d36d0be13eedb408c7b1405107f384e87bb5

# 关闭语义：关掉最后一个 pane/tab 即退出 + 死 pane 点击关闭

## 背景（Context）

关掉最后一个 pane（或 tab）原先会自动 `do_new_tab` 复活一个 shell，用
"关闭"这个动作永远离不开应用；带装饰窗口时还能用 WM 关闭按钮，无边框后
没有系统标题栏，退出语义必须显式（Ctrl+Shift+Q 已在上一批补上）。另外
pane 里子进程退出后尸体留在屏幕上（残留的 DEC 鼠标模式还会吃掉点击），
会话恢复时还存在跨 tab 的焦点泄漏。

## 变更摘要（Change Summary）

- 最后关闭 = 退出：`actions::do_close_pane` / `do_close_tab` 在 tabs 变空
  时不再 `do_new_tab`，改置 `UiState.quitting = true`；`screen.rs` 的空
  tabs 兜底加 `&& !d.ui.quitting`，避免这一帧又刷出新 tab；`main.rs::ui`
  每帧 `if ui.quitting { ViewportCommand::Close }` 直到关闭落地（关闭路
  径照常走 `save_if_dirty`，状态已落盘）。
- 死 pane 点击关闭：`screen.rs` 定义 `dead`（session 缺失或
  `exit.is_some()`）；死 pane 上单击 -> `do_close_pane`。鼠标追踪判定改
  成 `tracking = !dead && is_mouse_tracking(...)`：残留的 DEC 鼠标模式不
  再拦截关闭点击；活着的 mouse-tracking 应用行为不变（右键仍归它，
  context menu 仍被抑制）。
- 恢复聚焦修正：persist 的全局 id remap 可能把"别的 tab 的 pane id"解析
  成本 tab 的合法 id，焦点因此跨 tab 泄漏；改为按 tab containment 过滤
  （`layout_tree::contains_pane`），不命中落回 `first_leaf`。恢复后
  `active_tab = 0`（重建用的 `new_tab()` 会把最后一个 tab 设为 active）。
- 测试：`close_tab_tests`（单 pane tab 关闭置 quitting；两 tab 关最后
  一个也置 quitting）；persist `stale_cross_tab_focus_resolves_to_own_leaf`。
- e2e（e2e-oc-exit.sh）：K4 非最后 pane 上 Ctrl+Shift+W 关闭后应用存活；
  K6 单击死 pane 只关它自己，其它 pane 照常；K5 全局退出不变。

## 影响面（Impact Surface）

- 用户可见：关最后一个 pane = 退出应用；死 pane 单击即可清理（不必先
  Ctrl+Shift+W）；恢复后的按键不再投进别的 tab。
- Ctrl+Shift+W 在非最后 pane 上仍是"关 pane"，在最后一个 pane 上 = 退出。
- 无退出确认对话框，与终端关闭习惯一致。

## 备注（Notes）

- `quitting` 是"关闭落地前"的守卫：egui 的 Close 命令要到本帧结束才生
  效，守卫防止空 tabs 帧里兜底逻辑再 spawn 一个 shell。
- 死 pane 判定覆盖 spawn backoff（session 尚未建立）的情形；关闭后
  `do_close_pane` 移除 pane 并 `continue`，同一帧不会重复处理。

## 相关文档（Related Docs）

- `features/changelog/2026-09-06/exit-keys-title-centering-quit.md`（Ctrl+Shift+Q）
- `features/changelog/2026-09-10/borderless-translucent-window.md`（同批窗口壳）
