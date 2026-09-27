Commit: 832f94ebca7792accf3a918f29d42cc130fb075b

# e2e 套件覆盖索引 / e2e suites - coverage index

两层结构：先给每个套件一句中文速览（快速通读），再给从
[agents.md](../agents.md) 原样搬出的英文明细（脚本路径、阶段编号、参数
保持原文不翻译，避免丢失细节）。套件名/阶段编号被
[verified-end-to-end.md](verified-end-to-end.md)（已实测能力）与
`features/changelog/*` 引用；crate 地图、构建门槛与硬知识仍在
[agents.md](../agents.md)。

## 中文速览

- headless UI smoke：Xvfb + xdotool 基础存活/输入烟雾（无 WM 时必须
  `xdotool windowfocus` 才能把按键送进窗口）
- remote keeper：`cargo test -p remote --test keeper_selection` 验证远端命令
  只调用 `terminator-session`；`cargo test -p terminator-session --test reconnect`
  用私有 HOME 验证断开、重连、并发附着及 shell 状态存活。
- 真断网后的 SSH 全链路测试尚未在当前 keeper 实现上重建；验证时须隔离
  测试连接，不能停止共享 sshd 或触碰已有会话。
- control-channel e2e（`e2e-ipc-oc.sh`）：list/capture/send +
  oc link/submit/status/--wait + SIGTERM 后陈旧 socket 回收
- notice 气泡 e2e（`e2e-notice.sh`）：`terminator-ctl notice`（无参数，
  任何参数 = 用法错误）把规范 OSC 9 字节写进所在 pane 的 /dev/tty →
  后台 tab 的 chip 蓝点像素出现、重复 notice 去重、
  点击 chip 激活该 tab 即确认清除、激活 tab 聚焦时抑制、切回非激活后
  蓝点同位重现；裸 printf 同样点亮；keeper 案例：pane 经
  `terminator-session attach` 附着后 notice 字节穿过 keeper 照样点亮
- mouse/key e2e（`e2e-mouse-key.sh`）：裸 Ctrl+C 的 ^C 回显、跨 pane 拖拽
  SGR 投递到按下者、Shift+PageUp/End 翻页、Ctrl+C 真打断前台任务
- multi-window e2e（`e2e-windows.sh`）：Ctrl+Shift+N 真开第二个 X 窗口、
  跨窗口输入隔离、关最后 pane 即关窗口、W6-W10（含跨窗口 chip 拖拽）
- window-chrome e2e（`e2e-window-controls.sh`）：需要 OPENBOX 的边角
  resize / 最大化 / 最小化 / tab 溢出滚动 / 关闭 X 弹确认框（R6：Esc、
  再点一次 X、Cancel 取消，Quit 退出全部窗口）
- tab-close e2e（`e2e-tab-close.sh`）：Xvfb + openbox 下实际点击 tab X、
  中键、确认与取消；覆盖多 pane tab、次级窗口、最后一个 tab，以及终端
  标题栏 X 的直接关闭，并核对持久化 tab 与存活 pane 数量。
- drag-and-drop e2e（`e2e-dragdrop.sh`）：tab 排序、Ctrl+拖 pane 头部
  5 区投放（含拖拽中 overlay 像素断言）、跨 tab chip-dwell 迁移
- empty-window restore e2e（`e2e-empty-restore.sh`）：`windows:[{tabs:[]}]`
  恢复到活 tab、退出自动关 pane 且持久化空 tabs
- IME e2e（`e2e-ime.sh`）：真 XIM 链路（Xvfb + dbus + ibus + libpinyin）；
  M1 组合被吞、M2 preedit 墨迹、M3 空格上屏 汉字、M4 tab 改名第一个键、
  M5 面板改名 + 编辑器关闭后首个键、M6 Shift 切英文直通、M7 退出
- opencoder exit e2e（`e2e-oc-exit.sh`）：真 opencoder 二进制；Ctrl+D /
  Ctrl+C 退出后 pane 自动关闭、非最后一个 pane 的 Ctrl+Shift+W 不退出
- cjk font e2e（`e2e-cjk.sh`）：两套内嵌字体 cmap 覆盖 + 真实汉字墨迹
  像素（含 tofu 空心判据）
- ui style e2e（`e2e-ui-style.sh`）：scrot/PIL 像素门（pane 底色/主题混合、
  gutter 双色、chrome 顶栏、tab 下划线 + I1/I2 单 tab 仍显芯片行）

## 明细

- headless UI smoke: Xvfb :NN + xdotool (type into window works; needs
  `xdotool windowfocus` - no WM focus otherwise)
- remote keeper: `cargo test -p remote --test keeper_selection` and
  `cargo test -p terminator-session --test reconnect` use private fixtures;
  no shared SSH connection or existing process is changed.
- control-channel e2e: `scripts/bin/e2e-ipc-oc.sh` (Xvfb + fake opencoder
  holding a fixture store open in a named pane; covers list/capture/send,
  oc link/submit/status/--wait, stale-socket reclaim after SIGTERM)
- notice bubble e2e: `scripts/bin/e2e-notice.sh` (Xvfb + scrot/PIL + ctl
  + terminator-session; `terminator-ctl notice` (NO arguments, any arg =
  usage error) writes the canonical OSC 9 bytes
  `\x1b]9;terminator-rust notice\x07` from ipc_proto::notice_osc() to the
  pane's own /dev/tty - ordinary PTY output, so no keeper is needed for
  the core flow; N1 = a notice typed in a PLAIN pane on a NON-selected
  tab paints the blue dot rgb(70,150,255) right of the active chip and
  prints NOTHING on success (capture asserts silence + no rendered OSC
  garbage), N2 = repeated notices dedup to ONE dot cluster, N3 =
  `terminator-ctl notice foo` is a usage error (rc!=0, empty stdout,
  badge unchanged) while the bare command always exits 0, N4 = clicking
  the badged chip switches active_tab (persisted to state.json) and the
  focused window's acknowledge clears the dot, N5 = noticing from the
  ACTIVE focused tab is suppressed/acked (zero clusters),
  N6 = backgrounding beta again re-lands the dot right of the active
  chip at the SAME x, N7 = the zero-binary fallback
  `printf '\033]9;terminator-rust notice\007'` paints and acks the same
  way, N8 = after `terminator-session attach notes --title notes` INSIDE
  pane 13 a notice typed in the keeper shell still lands the badge (the
  bytes traverse the keeper as ordinary PTY output; private socket +
  `list` STATE=attached asserted first), N9 = app + keeper liveness).
- mouse/key e2e: `scripts/bin/e2e-mouse-key.sh` (Xvfb + xdotool over the
  live socket: bare Ctrl+C ^C echo, cross-pane drag SGR press/motion/
  RELEASE landing in the press-owner pane, Shift+PageUp/End scrollback
  paging, Ctrl+C actually interrupting a foreground job)
- multi-window e2e: `scripts/bin/e2e-windows.sh` (Xvfb: Ctrl+Shift+N
  spawns a real second X window, cross-window typing isolation via ctl
  capture, last-pane close removes the window, re-spawn, quit-from-
  secondary kills the app, W6 root last-pane close with a sibling alive
  respawns a fresh root tab, W7/W8/W9 Ctrl+Shift+J move + single-window
  split + Ctrl+Shift+M merge, W10 = chip drag onto the OTHER window's
  strip: source window dies, pane keeps its pid and lands ACTIVE in the
  root). W10's pixel gates are THEME-AGNOSTIC (DEFAULT = kanagawa-wave):
  chip_edges = FIRST wide non-bare-chrome run (chips left-aligned, the
  trailing chrome merges into a LONGER run), strip_diff = strip band vs a
  baseline scrot (never e2e-lib's dracula fill scan). CI runs it (zig via
  PyPI + fetch-vendor.sh; fmt/clippy hard gates) + e2e-dragdrop.sh
  (scrot + python3-pil). Rare viewport-churn flake: egui-wgpu
  staging-buffer / Dropped-frame validation errors (~1/30, upstream).
- window-chrome e2e: `scripts/bin/e2e-window-controls.sh` (Xvfb + OPENBOX -
  bare Xvfb has no WM so Maximized/Minimized/BeginResize/StartDrag are all
  EWMH no-ops; R1 edge-drag resize, R2/R3 maximize button + chrome
  double-click toggle, R4 = minimize glyph scrot/PIL probe (dash ink
  centroid ON the row centre, theme-agnostic lum - bg weight: a lowered
  dash reads '_'; ui/tabs_widgets.rs edge_cells draws the dash at
  min_rect.center(), not y+4) THEN iconic + windowactivate restore,
  S1-S3 tab overflow: chip strip scrolls by wheel while '+"/edge cells
  stay pinned at CHROME_RESERVE=173; R6 close X opens a centered
  Quit/Cancel confirm modal (one click; the scrot diff must show ONE solid
  centered frame, keys stay out of the pane, Esc / a re-click on X /
  Cancel dismiss, Quit quits); CI job
  e2e-window-controls apt adds openbox, scrot and python3-pil)
- tab-close e2e: `scripts/bin/e2e-tab-close.sh` (private HOME + Xvfb +
  openbox; click-driven tab X / middle-click confirm, Escape cancellation,
  multi-pane session termination, direct pane-header X, secondary-window
  isolation and last-tab quit; CI runs it after e2e-window-controls).
- drag-and-drop e2e: `scripts/bin/e2e-dragdrop.sh` (Xvfb + xdotool +
  scrot/PIL + state.json tree asserts; D1/D2 = Ctrl+drag pane header to
  sibling edge/center with mid-drag overlay pixel checks, D3 = chip
  reorder persisted, D5 = plain header drag, D6/D7/D8 = cross-tab pane
  migration via chip dwell (edge/Center-swap/source-tab-close), D4 = alive
  + quit).
- empty-window restore e2e: `scripts/bin/e2e-empty-restore.sh`
  (Xvfb + xdotool; E1 = `windows:[{tabs:[]}]` restores a live tab,
  E2 = exit auto-closes + app quits persisting empty tabs, E3 = the
  loop relaunches live; wired into ci.yml as e2e-empty-restore).
- IME e2e: `scripts/bin/e2e-ime.sh` (REAL XIM chain: Xvfb + dbus
  session bus (private fork fallback) + ibus-daemon --xim + engine
  libpinyin + LANG=zh_CN.UTF-8; M1 composing 'hanzi' never reaches the
  pty, M2 preedit ink purple-hue-detected vs a baseline scrot, M3 space
  commits 汉字, M4 tab-chip rename composes 汉字 from the FIRST key,
  M5 pane-header rename + the pane's first key after the editor composes
  鸟, M6 bare Shift_L toggles libpinyin EN for ASCII passthrough (LAST,
  composing phases need CN), M7 liveness + Ctrl+Shift+Q quit;
  CI job e2e-ime apt: ibus ibus-libpinyin dbus x11-utils locales +
  locale-gen zh_CN.UTF-8 - XIM locale negotiation needs it).
- opencoder exit e2e: `scripts/bin/e2e-oc-exit.sh` (Xvfb + REAL
  /root/opencoder binary; OC_BIN override; SHELL wrapper that `exec`s the
  binary so pane pid == opencoder pid -> `kill -0` is exit ground truth; dummy
  ~/.opencoder/config.json needed - onboarding form eats ^C; focus navigation
  via Ctrl+Shift+Right, NEVER clicks into mouse-tracking panes; K4 =
  Ctrl+Shift+W on a NON-last pane keeps the app alive (quit only on the
  last close); opencode FIRST RUN seeds
  ~/.opencoder (skills installer + state dirs) - concurrent first-runs
  RACE it and instances exit(1) silently after "first frame" (repro: new
  HOME x3 pty = 2 dead, warm HOME = 3/3 alive), so the script WARMS UP the
  scratch HOME with one throwaway pty run before launching the app)
- cjk font e2e: `scripts/bin/e2e-cjk.sh` (fontTools cmap coverage of BOTH
  embedded subsets + a 2:1 advance probe on the Maple one; Xvfb live app: ctl
  send `echo 汉字测试` capture round-trip + scrot/PIL connected-ink
  assertions - real Han ink ~11-13x12-13px with interior strokes; ASCII
  <=8x12, wide filter 8.5..22 x 10.5..22, tofu square is hollow inside)
- ui style e2e: `scripts/bin/e2e-ui-style.sh` (Xvfb + scrot/PIL pixel
  assertions: pane bg/theme blend via transparency, gutter two-tone
  (chrome_bg field + 2px rounded grab handle, hover step mix 0.22 only
  under the pointer), chrome top-bar fill, active-chip underline (rounded
  caps, inset past the pill corners) + fill; expectations are COMPUTED
  in-script from dracula constants via a mix() helper - token retunes
  touch only colors.rs/tokens.rs, geometry changes touch the script;
  presets a dracula Split state.json (pane bg/transparency now live in
  settings); checks I1/I2 assert the chip row STAYS with a single tab:
  chip fill at (X+50,Y+7 - PROBE ABOVE THE INK: the 15pt "style [2]"
  label ends ~X+89 and the close X starts ~X+96, the label/close gap is
  too tight to sample; the old X+50,Y+15 only passed by landing on the
  label's space char), underline band Y+30..Y+31, pane header tint
  pushed down to Y+44..Y+64, content from ~Y+66)
