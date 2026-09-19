# Tab chip 拖拽排序 + Ctrl+拖拽 pane 头部移动（5 区投放）

## 背景（Context）

多 pane/多 tab 场景下顺序调整只有键盘捷径，缺少直接操作手势。落地两个
拖拽手势：tab chip 拖拽排序（ghost chip + 实时 move_tab），Ctrl+主键拖
pane 头部移动 pane（5 区投放 overlay：中心 50% 方块 = 交换，否则最近边
= 脱离 + 按 settings.split_ratio 重切分）。

## 变更摘要（Change Summary）

- layout-tree：新 `src/movepane.rs` —— `zone_for`（中心方块 ±0.25 判
  Center，否则最近边）、`zone_rect`（预览区域，ratio 钳 0.05..0.95）、
  `move_pane_to_pane`（edge = remove_pane_node + split_pane 重挂；
  Center = 两 leaf id 原位互换，形状/ratio 不变）；`tabs.rs::move_tab`
  记忆锚点 pane，active_tab 跟随被移动的 tab。
- app/ui/tabs.rs：chip 改 `Sense::click_and_drag()`，drag_started 锁存
  `WindowUi.tab_drag`（anchor = tab 最低 pane id，免疫重排引起的 index
  漂移）并选中该 tab；被拖槽位留空隙，ghost chip（选中态样式）钉在指
  针下、越过其它 chip 中心时逐帧 `move_tab`；主键释放结束。锁存期间
  chrome_drag 不再 StartDrag（不拖走 OS 窗口）。
- app/ui/pane_header.rs：Ctrl+主键 press 把头部拖拽转为 pane 移动锁存
  `WindowUi.pane_drag`（普通拖拽仍 StartDrag 移动 OS 窗口）；screen.rs
  锁存期间压制 divider 交互与原始指针路由，每帧用**全量** pane rect（含
  头部）刷新投放目标，释放时 `actions::do_move_pane` 落盘。
- overlay `render/dropzone.rs`：实心 mix 阶梯色（确定性像素）——目标
  pane 1.5px 环 mix(bg, block_highlight, 0.55)；预览区填充 0.22 + 2px
  描边 0.85（R_MD 圆角）；源 pane 1px mix(bg, fg, 0.35)。画在 divider
  之后。
- 顺序/结构经 state.json 持久化（既有 dirty 保存路径，无新 schema）。

## 验证（Verification）

- 单测：layout-tree movepane/tabs 与 app actions::do_move_pane
  （edge 重切分用 settings.split_ratio、Center 交换、非法目标拒绝且不
  变）。
- 新 e2e `scripts/bin/e2e-dragdrop.sh`（Xvfb + xdotool + scrot/PIL +
  state.json 断言）：D1 拖 pane 头至右半边缘区，中途 scrot 断言预览填
  充 mix(bg,accent,0.22) 只在右侧半区、释放后 root=v(2|1)；D2 中心区
  释放后 id 原位交换回 v(1|2)；D3 chip 拖过第二个 chip 后 titles==
  ["beta","alpha"] 且被拖 tab 保持选中；D4 存活 + Ctrl+Shift+Q 退出。
  修复 press/move 合帧竞态后 16/16 全绿。
- 回归：`e2e-ui-style.sh`（静止 tab 栏像素无拖拽痕迹）与
  `e2e-mouse-key.sh`（跨 pane 指针路由）ALL GREEN；fmt/clippy/
  build/test --workspace 全绿。

## 注意（Notes）

- 头部是 `Sense::drag()`（非 click），press 帧即 drag_started，Ctrl 判
  定读当帧 modifiers；chip 是 click_and_drag，egui 按位移 >6px / 时长
  >0.8s 才判定 drag（e2e 用分步 mousemove 驱动）。
- e2e 稳定性：mousedown 与第一步 mousemove 之间必须 settle ~0.3s——
  egui 每帧按 pointer.latest_pos() 做命中测试，press 与移动合并进同一
  帧会把拖拽锁到错误的 chip（或裸 chrome → StartDrag）；修前 D3 约
  1/9 概率翻车，修后 16/16 绿。
- move_tab 后 active_tab 指向被拖 tab 的**新** index（锚点跟随），非
  固定 0。
- CI：`.github/workflows/ci.yml` 新增 e2e-dragdrop job（xvfb + xdotool
  + scrot + python3-pil），与 e2e-windows 同构。
