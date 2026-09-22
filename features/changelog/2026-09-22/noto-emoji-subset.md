# Noto Emoji subset: 🤖 avatar fell through the font chain

## Problem
opencode 欢迎页/提示面板的机器人头像 🤖 (U+1F916) 渲染为 `?` 且右侧留一个
空白格（用户报告的"乱码 + 断层"，同一个 bug 的两个症状：🤖 是双宽格
2 cell ≈18px，`?` 只占左半 → 右半是洞）。

Root cause: egui 0.36 内置 NotoEmoji-Regular.ttf 恰好缺 U+1F916 和
U+FE0F（其余 opencode 用到的 emoji 👋💡💭📋🎮🎉🌍👧👨👩🇹🇻 内置字体都有），
整条链（Maple → egui 内置 → NotoSansSC → TerminalSymbols）无人兜底，
epaint 对无字形字符画 `?`。欢迎页 👋 行丢 2 格缩进是 opencode/ratatui
上游自己从 col2 开始写（pty 抓流证实），不是终端 bug。

## Fix
- 新增 `assets/fonts/NotoEmoji-subset.ttf`（364KB, 605 字形, OFL，
  `NotoEmoji-OFL.txt`）：源 = google/fonts `ofl/notoemoji/NotoEmoji[wght].ttf`
  （cdn.jsdelivr.net 可达；github/raw 被墙），先 varLib.instancer 实例化
  wght=400（pyftsubset 无 --instancer），再 pyftsubset
  `--no-layout-closure --no-hinting --unicodes-file=<egui-emoji cmap 的补集>`
  —— 只装 egui 内置 emoji 没有的 605 码点（🤖、FE0F、20E3 除外等等），
  风格与 egui 内置 NotoEmoji 完全一致。
- `crates/app/src/ui/fonts.rs`：`terminator-emoji` push 到两条链的
  LAST（在 terminator-symbols 之后），只补洞不抢已覆盖字形；无 env
  覆盖（与 symbols 字体一致）；单测扩到 4 字体 parse_ok + 链尾断言。
- `scripts/bin/e2e-cjk.sh`：cmap 门加 NotoEmoji-subset 样本
  `🤖🤔🧠🫡+FE0F`；live 输入行改为 `汉字测试🤖`，capture 断言 🤖。

## Verification
- e2e-cjk.sh ALL GREEN（cmap 609 glyphs、capture 含 🤖、像素门不受影响）。
- 真 opencode（/root/opencoder release, dummy config + 预热 HOME）Xvfb 截图
  像素复验：🤖 行首 span 28..43 = 16px 密集字形铺满双格（86 墨点/33%），
  右半格 37 墨点（修复前 0），后随 `Rust` 文本无缝衔接。
