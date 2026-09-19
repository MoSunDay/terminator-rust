# UI 现代化：令牌层 / 浮层立体 / pill chip / 焦点卡片 / 微动效

## 背景（Context）

调查阶段产出 5 阶段 UI 现代化方案并全部落地：浮层立体化、设计令牌层、
chrome 精修（chip pill 化 + 焦点态重构 + divider 把手化）、微动画（含
保险开关）、细节打磨。硬约束：文件 ≤400 行（超出需拆分）、每阶段
fmt+clippy（CI 硬门禁）+ workspace 测试、纯函数式无 class、e2e 期望
同步、几何改动跑 mouse-key 回归。

## 变更摘要（Change Summary）

- 阶段 1 浮层立体化：style.rs 安装 window_shadow/popup_shadow 柔和
  阴影，window_corner_radius=R_LG、menu_corner_radius=R_MD；inspector
  四段弱化分区标题。
- 阶段 2 设计令牌层：新 `render/tokens.rs`（76 行，纯函数/常量）——
  圆角刻度 R_SM/R_MD/R_LG、shadow(Layer) 预设、hover_t/lerp_color/
  cursor_alpha/motion_enabled。colors.rs 未塞新代码。
- 阶段 3a chip 精修 + 拆分：chip 全角 6px pill、2px 圆头下划线（内缩
  过 pill 圆角）、关闭钮圆形 hover 底；tabs.rs 395→244 行，chip 绘制
  拆出 `ui/tabs_widgets.rs`（244 行）守住 400 行上限。
- 阶段 3b 焦点态 + divider：聚焦 pane 1px accent 描边 + header 左 3px
  竖条，非焦点 hairline 卡片；divider 2px 圆头把手、hover mix 0.22
  仅在指针下，命中区 6px 不变。
- 阶段 4 微动画：光标 alpha 正弦软闪（仅聚焦 pane，grid DrawArgs.
  cursor_alpha + lerp_color）、hover 120ms 淡入；
  `TERMINATOR_NO_MOTION=1` 钉死（6 个 Xvfb e2e 脚本均导出）。
- 阶段 5 打磨：viewport bar 2→3px 圆头 thumb（mix 0.25）、dead pane
  标题弱化、右缘格子 hover rounding 6。
- e2e-ui-style 期望同步：B 把手两色（静止/hover 0.22）、E 圆头下划
  线、指针停靠裸 chrome + 0.5s settle（Xvfb 初始指针在屏心恰压
  gutter；50ms 重绘节拍）。

## 测试与验证（Test & Verification）

- 实现阶段门禁全绿：`cargo fmt --all --check` 0 差异；clippy 0 告
  警；`cargo test --workspace` 全过（67+31+20+27+1 passed）。
- e2e 全量：ui-style A-I 全 OK、mouse-key T1-T4、windows W1-W6、
  cjk、ipc-oc、oc-exit K1-K6 全绿（均 NO_MOTION=1）；inspector Xvfb
  冒烟 0 panic。
- 本次提交会话按用户指令免测，仅复核 `cargo build --workspace
  --all-targets` 通过（改动自上一轮验证后无新变更）。

## 影响面（Impact Surface）

- 视觉层全面翻新但几何不变：cell 网格、鼠标命中区、divider 拖拽
  6px、按键路径零扰动（mouse-key 全量回归证实）。
- 新增环境开关 TERMINATOR_NO_MOTION（钉死 hover 淡入与光标软闪），
  Xvfb 无合成器环境的像素断言依赖它。
- ctl/IPC/多窗行为不变。

## 备注（Notes）

- 方案唯一偏差：epaint 0.36 Stroke 无圆头线帽，右缘格子“1.5px 圆头
  描边”不可实现，保留 1.5px 平头描边 + hover rounding 6。
- 阴影/透明在真机合成器下的观感验收（部署目标 192.168.31.196）仍
  待做——Xvfb 无合成器无法像素验证，仅观感确认。

## 相关文档（Related Docs）

- `agents.md`（ui style e2e / Xvfb 指针 / motion 开关条目）
