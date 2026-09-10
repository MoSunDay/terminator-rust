Commit: f3d7c9975f75116eaab5733b74660350630aa1c2

# 内置 CJK 字体跟进：谚文音节子集化 + env 字体启动预校验

## 背景（Context）

5f77d36 评审遗留 TODO（两项 P2）：① `TERMINATOR_CJK_FONT` 指向可读但
损坏的字体文件时，epaint 在首次 layout 处 panic、应用起不来
（fs::read 成功 != 可解析）；② 原始要求"中文、unicode 等正常展示"，
韩文音节 AC00-D7AF 未子集化会 tofu，仅靠 env 换全量 TTC 逃生门补偿
不足。另按门禁纪律补跑评审跳过的 e2e-ipc-oc / e2e-mouse-key（P3）。

## 变更摘要（Change Summary）

- 资产重子集：U+AC00-D7AF 并入（11172 个谚文音节，组件合成字形），
  8169076B/29158 字形 -> 9785240B/40330 字形（+1.6MB，低于评审估算的
  +2.5MB）。复现配方（先精确复现旧资产 29158 字形/cmap 全等）：SC 面
  （ttc idx 2）+ 原终端区段（另含 U+2015/引号/省略号、U+3190-319F
  竖排注记）+ `--no-layout-closure`（默认 GSUB 闭包会膨胀到 47k 字形）。
  不变量保持：upm=1000、'汉' advance=1000（宽字缩放探测不受扰）、
  cmap 无 ASCII（Latin 主字体零扰动）；谚文 advance=920（0.92em），
  ghostty 按 EAW 标宽格，字形在两格对内略窄（与真实终端观感一致）。
- `ui/fonts.rs`：新增 `parse_ok()`，用 skrifa `FontRef::from_index`
  （epaint 0.36 `Fonts::new` panic 路径的同一解析器）在
  `env_font_data()` 内预校验；读不到或解析失败一律 log::warn +
  回退内置字体。skrifa 0.44 升为 workspace 依赖（Cargo.lock 原本已含，
  经 epaint 传递，无版本扰动）。4 个密闭单测：内置字体 idx0 通过 /
  垃圾字节 / 越界面索引 / 空字节。
- README：新增 Fonts/Unicode 小节（覆盖范围、TERMINATOR_CJK_FONT
  语法、预校验回退行为、OFL 指引）。
- `e2e-cjk.sh`：cmap 样本集 += 한글（资产级确定性门禁）；实弹像素
  断言仍以 Han 为基准（谚文与汉字共用同族链回退与 EAW 宽格模型）。

## 影响面（Impact Surface）

- 二进制 +1.6MB（release ~38.5 -> ~40MB）；网格几何/PTY/chrome 零
  扰动（宽字缩放探测源 '汉' 的 advance 与 upm 未变）。
- 验证：workspace 207 单测 + clippy 0 warning；e2e-cjk /
  e2e-ui-style / e2e-ipc-oc / e2e-mouse-key 四门禁全绿。
- 仍未闭合：macOS 实机冒烟（本环境无 Apple SDK；"内置字体 + 零平台
  分支"已结构性消除主要风险）；Ext B（20000+）仍需 env 换全量 TTC；
  IME 中文输入链路未验证（展示 != 输入）。
