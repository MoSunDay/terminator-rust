Commit: 5f77d36d0be13eedb408c7b1405107f384e87bb6

# 内置 Noto Sans SC 子集，中文/宽字符跨平台正常展示

## 背景（Context）

用户要求 Linux 与 macOS 上中文、Unicode 均正常展示。现状：app 只用
egui 默认字体（Ubuntu-Light/Hack/NotoEmoji），无任何 CJK 字形，中文
渲染为豆腐块；ghostty 网格侧的宽字符模型（wide 标记 + spacer 尾格）
早已在位，缺的只是字形覆盖与宽字绘制对齐。用户进一步明确：内置字体、
跨平台支持（不依赖系统字体安装）。

## 变更摘要（Change Summary）

- `assets/fonts/NotoSansSC-Regular-subset.otf`（8.1MB，OFL，见同目录
  OFL.txt）：从 Debian noto-cjk 包的 NotoSansCJK-Regular.ttc 提取 SC
  字面（index 2），fontTools 子集化到终端常用区段——CJK 部首/标点
  2E80-303F、假名 3041-30FF、注音/谚文兼容 3105-318F、括注
  3200-33FF、扩展 A 3400-4DBF、基本区 4E00-9FFF、兼容表意 F900-FAFF、
  竖排形式 FE30-FE4F、全角形式 FF00-FFEF、罗马数字 2160-2183、带圈
  2460-24FF（29158 字形，全角步进精确 1em）。
- `crates/app/src/ui/fonts.rs`（新）：`include_bytes!` 内置字体 +
  `TERMINATOR_CJK_FONT=path[:face_index]` 环境变量替换（TTC 面索引，
  最后一个冒号且后缀可解析 u32 才算索引）；追加到 egui
  Monospace/Proportional 家族链末尾做字形回退（幂等），启动时
  `Terminator::new` 里 `ctx.set_fonts` 一次安装。
- 宽字绘制对齐：CJK 字形 1.0em 步进 vs 窄格 'M' ~0.6em，宽字必须恰好
  占两窄格。`CellSize.wide_size`（state.rs）记录测量出的缩放字号；
  `measure_cells` 用 `layout_no_wrap("汉")` 沿同一族链探测全角步进，
  scale=2w/advance（实测 ~1.204，钳 0.8..=1.8，退化兜底 1.2）；
  `draw_frame` 宽格用该字号绘制，窄格路径字节级不变。
- 测试：fonts 纯函数（spec 解析/幂等追加）；grid 不变量（headless
  `ctx.begin_pass` 初始化 atlas，宽字 advance == 2*w ±0.05）；vt-pane
  `cjk_wide_cells_flagged_with_spacer_tails`（汉a字 -> wide/spacer/
  cursor+5）。
- `scripts/bin/e2e-cjk.sh`（新）：fontTools 校验资产 cmap 覆盖样本集；
  实弹 Xvfb 起应用（坏字体会在 epaint 解析时 panic，起不来即失败），
  ctl send `echo 汉字测试` 经真实 pty，capture 精确回读；scrot+PIL
  连通域断言——真实汉字墨迹 ~15x17px 且有内部笔画（ASCII ≤9x10.5、
  tofu 替身方块内部空心），≥4 个宽字模 + ≥3 个带笔画。Xvfb 探测随机
  空闲 display（固定/PID 派生号会撞残留 socket）。

## 影响面（Impact Surface）

- 所有 egui 文本（终端格、pane 标题、tab 名）获得 CJK 回退；Latin
  主字体不变，'M' 步进不变 -> 网格几何/PTY resize 无扰动。
- 二进制 +8.1MB（release 30->38MB）；e2e-ui-style 八项像素断言全绿
  （chrome 颜色与标题居中不受影响）。
- macOS/Windows：同一份内置字体随二进制走，无需系统字体，路径探测/
  cfg 分支为零；用户可用 TERMINATOR_CJK_FONT 换全量 TTC（如繁体/韩文
  面索引）。
