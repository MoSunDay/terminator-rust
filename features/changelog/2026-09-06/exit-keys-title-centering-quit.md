Commit: a89efa6a52338c50a56abdacc9eed9b37b401f1f

# 退出键 + 标题居中 + 全局退出（kitty-flag 修复 / Ctrl+Shift+Q / 像素断言）

## 背景（Context）

子进程 push kitty keyboard flags 后（crossterm 0.28 类 TUI，真实 opencoder 推 flags 7），
vendored ghostty key encoder 会丢弃所有无 utf8 text 的 Ctrl+letter（flags=1 即可复现：
Ctrl+D 编码为 `[]` 而非 `[4]`），导致 ^C/^D 无法退出 TUI。同时应用没有任何全局退出
快捷键（`SKey::Q` 不可构造、无 `Action::Quit`），pane/tab 标题非几何居中，rename
编辑器点击编辑框外部不会取消（焦点陷阱：编辑态锁死全部快捷键）。

## 变更摘要（Change Summary）

- `vt-pane/src/task.rs` send_key：三重门（mods 精确等于 CTRL、字母键 A–Z、
  term 当前 kitty flags 非空）命中时仅对 ENCODER OPTIONS 清空 kitty flags；
  terminal 真实态零接触，下一键 `set_options_from_terminal` 自动恢复子进程 flags；
  flags 读取 Err 保持旧行为（保守，无新失败模式）。
- 字节级回归 `crates/vt-pane/tests/kitty_ctrl_bytes.rs`：真 pty 子进程
  （`printf flags; stty raw -echo; cat > file`）记录对端原始字节，5 组断言钉死契约：
  flags7+Ctrl+D→`[4]`、flags1+Ctrl+C→`[3]`、无 flags→`[4]`、Enter flags7→`[13]`、
  Esc flags7→`ESC[27u`。
- 全局退出：`SKey::Q`（keymap `to_skey` 补构造入口）→ `Action::Quit`
  （shortcuts 路由，ctrl-only / shift-only 落 None 有单测 pin）→ keyboard.rs 发
  `ViewportCommand::Close` + continue（不透传 pty；与 WM-close 共享
  `close_requested` → `save_if_dirty` 持久化路径）。
- 标题居中：pane_header 以 `painter.text(title_rect.center(), CENTER_CENTER)` 几何
  居中；tabs.rs `TITLE_ICON_ZONE=41`（5+16+4+16）扣除右侧图标区后光学居中。
- rename 编辑器外点取消：pane_header / tabs 均以 `any_click() && interact_pos`
  不在编辑矩形判定，`reason=None` 取消（不触发重名校验）；与按钮同帧双触发
  （取消编辑 + 触发按钮）为备案取舍，优先关死焦点陷阱。

## 影响面（Impact Surface）

- 普通打字不受累：shift+Q 落 None；`send_char` 恒 `Mods::empty()` 不进门；
  mouse 自持独立 encoder（自带 `set_options_from_terminal`），flag 清除不泄漏进 SGR。
- e2e：`scripts/bin/e2e-oc-exit.sh`（K1–K5：真实 opencoder，`exec` wrapper 使
  pane pid == opencoder pid，`kill -0` 为退出真值；K5 带 `kill -0` 预检）+
  `e2e-ui-style.sh` 新增 G/H 像素断言（pane 标题 / top-bar 标题中心，期望由
  几何常数在脚本内同源计算）。

## 语义与边界（Notes / Compatibility）

- 编辑器聚焦期间全部快捷键（含 Ctrl+Shift+Q）不可达（`egui_wants_keyboard_input`
  早退，与所有文本框共有的既有约定）；先 Esc/外点取消再按。
- Ctrl+Shift+letter 在 kitty-flags 子进程下仍被丢弃（三重门取精确 CTRL：
  这些组合本非 C0 可映射，合成 CSI-u 超出 legacy encoder 能力，保守不修）。
- 严格 kitty-only 的消费者会改收 C0 字节：实际目标 crossterm 0.28 双格式通吃
  （raw-pty 已验证 0x03/0x04/`ESC[99;5u` 均干净退出）。
- 超长标题在收缩后的居中域内可与图标重叠（layout_no_wrap 不裁剪，旧行为同样
  会撞图标，无恶化）。

## 相关文档（Related Docs）

- [../../agents.md](../../agents.md)（repo 根，逻辑图与 hard-won facts）
