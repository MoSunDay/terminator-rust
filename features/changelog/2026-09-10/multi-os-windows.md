Commit: 18d846d1e0350487a2f22a10b3d97a2f2a041dbb

# 多 OS 窗口：每个窗口独立的 tab 树（egui immediate viewports）

## 背景（Context）

应用此前只有一个 OS 窗口：所有 tab / split 挤在一个 egui 视口里。用户要
求 terminator 式的多大窗口 —— 每个窗口各自容纳一组终端（tabs + splits），
窗口可独立关闭，全局退出仍然一键。egui 0.36 的 immediate viewport 在
eframe native wgpu 下是真实 OS 窗口（回调拿 `&mut Ui`，同步跑在该视口自
己的 input pass 里），根渲染路径可以整体复用。

## 变更摘要（Change Summary）

- 状态模型：`AppState.windows: Vec<WindowState{id, tree, WindowUi}>`；
  `active` = 正在渲染的窗口下标（渲染/按键处理期间指向本窗口），
  `focus` = 用户聚焦的窗口（viewport `focused==Some(true)` 时更新，
  IPC drain / inspector 跑在 `active = focus` 上）。每窗口自己的
  `WindowUi`（zoom、重命名编辑器、指针抓取、mods 种子、字号）。pane id
  全局唯一：`seed_alloc`/`collect_alloc` 跨窗口收账。
- 持久化：`PWindow{id, active_tab, tabs}[]`；旧单窗口文件的 `tabs` 键仍
  兼容（镜像成窗口 1）；第二+ 窗口重建时丢掉 `new_tree` 种子 tab（pane
  id 1 会撞车）再 `ensure_next_pane_id + new_tab`。恢复后 focus=0。
- `windows.rs`：`spawn`（Ctrl+Shift+N，全局窗口 id 从 1 递增，ViewportId
  = `Id::new(id)`，0 留给 ROOT）、`remove_window`（终结 pane + 收账 +
  retarget focus）、`render`（每窗口主体：本视口键盘处理 → 可选 tab 栏
  → CentralPanel screen）、`render_secondaries`（逐个
  `show_viewport_immediate`；槽位复检防漏窗）。
- 关闭语义：空 ACTIVE 树 —— root（idx 0）在有兄弟窗口时 respawn 新
  tab，仅剩它时置 `quitting`；secondary 直接自我移除（WM 关闭 = 只关该
  窗口）。窗口在渲染中途被移除时 `render` 提前返回，避免把下一个窗口的
  树画进垂死视口。Ctrl+Shift+Q 从任意窗口发
  `send_viewport_cmd_to(ROOT, Close)` 退出整个应用。
- e2e（e2e-windows.sh）：W1 Ctrl+Shift+N 出真 X 窗口且 pane id 全局唯
  一；W2 跨窗口打字隔离（ctl capture 断言 marker 互不泄漏）；W3 关
  secondary 最后一个 pane 只移除该窗口；W4 再生窗口照常工作；W5 从
  secondary Ctrl+Shift+Q 杀掉整个应用。

## 影响面（Impact Surface）

- 用户可见：Ctrl+Shift+N 开新窗口（900x600，级联偏移，OSC 标题照常接管
  标题栏）；窗口互不干扰；关一个窗口不影响其它；任务栏/WM 关闭只关单个
  窗口。
- 恢复（重启）只重建窗口几何与 tab 树；focus 落回窗口 1。
- IPC `list` 枚举所有窗口的 pane；`capture`/`send` 用 pane id 直达。

## 备注（Notes）

- 键盘事件只到达 X 聚焦的视口：每视口在自己的 input pass 里跑
  `keyboard::handle`，无需手工路由；`xdotool` 驱动有 WM 的远程桌面要
  `windowactivate`（windowfocus 不够），裸 Xvfb 里 windowfocus 即可。
- secondary 视口销毁比状态移除晚几秒（eframe GC），e2e 轮询 X 窗口消
  失而不是立刻断言。
- `import -window WID` 给 32 位 ARGB 窗口截图全黑：截 `root` 全屏再从
  里读窗口像素。
- 远程 state.json 里历史写入的 `opacity: 0.8` 不会被 serde default 迁
  移（只管缺键），已部署机上手工改回 1.0；透明仍是显式 opt-in。

## 相关文档（Related Docs）

- `features/changelog/2026-09-10/borderless-translucent-window.md`（同
  批窗口壳：无边框/透明/单 tab 零 chrome）
- `features/changelog/2026-09-10/quit-on-last-close.md`（单窗口时代的
  关闭语义，多窗口下 root 语义由本批扩展）
