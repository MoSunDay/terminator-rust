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
- remote e2e（`cargo test -p remote --test zellij_e2e -- --ignored`）：
  本机 sshd key + zellij 的远端会话；收尾只删本测试自己的 session，
  绝不 delete-all-sessions
- net-drop e2e：真断网后的自动重连全链路（退避重生、pending badge、
  marker 存活）；铁律是测试自身跑在 sshd 会话里 —— 不许停 sshd，只能
  kill -9 单条连接的子进程或用带注释的 iptables REJECT 窗口
- control-channel e2e（`e2e-ipc-oc.sh`）：list/capture/send +
  oc link/submit/status/--wait + SIGTERM 后陈旧 socket 回收
- mouse/key e2e（`e2e-mouse-key.sh`）：裸 Ctrl+C 的 ^C 回显、跨 pane 拖拽
  SGR 投递到按下者、Shift+PageUp/End 翻页、Ctrl+C 真打断前台任务
- multi-window e2e（`e2e-windows.sh`）：Ctrl+Shift+N 真开第二个 X 窗口、
  跨窗口输入隔离、关最后 pane 即关窗口、W6-W10（含跨窗口 chip 拖拽）
- window-chrome e2e（`e2e-window-controls.sh`）：需要 OPENBOX 的边角
  resize / 最大化 / 最小化 / tab 溢出滚动 / 关闭 X 弹确认框（R6：Esc、
  再点一次 X、Cancel 取消，Quit 退出全部窗口）
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

## 明细（English，从 agents.md 原样搬出）

- headless UI smoke: Xvfb :NN + xdotool (type into window works; needs
  `xdotool windowfocus` - no WM focus otherwise)
- remote e2e: `cargo test -p remote --test zellij_e2e -- --ignored`
  (needs local sshd key auth + zellij; cleanup deletes ONLY the test's
  own session via `zellij delete-session <name> --force` - the
  net-drop rule below spells out why never delete-all-sessions)
- net-drop e2e (REAL network interruption + session recovery):
  `cargo test -p remote --test net_drop_e2e -- --ignored` +
  `cargo test -p app reconnect_net -- --ignored` (the latter drives the
  full frame loop pump_all/close_exited/reconnect::pump/ensure_sessions
  against a live attach: close_exited spares the pane, pending badge
  shows, backoff(1)=1s respawn, marker survives). HARD RULE: the test
  process itself runs INSIDE an sshd session - NEVER stop/restart sshd
  or kill the listener; interrupt via kill -9 of the per-connection
  sshd child (pair ports with `ss -tnp`) or an iptables REJECT window on
  `-i lo --dport 22` tagged with a comment (idempotent delete; NEVER
  assert while the rule is inserted - a panic strands it). REJECT
  tcp-reset fails ssh in ~0.1s (ConnectTimeout never hangs); server RST
  yields a natural exit 255 both shapes reattach with state intact.
- control-channel e2e: `scripts/bin/e2e-ipc-oc.sh` (Xvfb + fake opencoder
  holding a fixture store open in a named pane; covers list/capture/send,
  oc link/submit/status/--wait, stale-socket reclaim after SIGTERM)
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
  e2e-window-controls apt adds openbox)
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
