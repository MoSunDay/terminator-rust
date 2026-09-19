# 主字体换装 Maple Mono NF CN：正文/图标/汉字一体渲染

## 背景（Context）

用户嫌终端正文"丑"：根因是 ASCII 用 egui 默认 Ubuntu-Light（细软无
锋），中文落在 Noto Sans SC 子集（平淡）。选 A 方案 —— Maple Mono
Normal NF CN：等宽、圆角现代字形、NF 图标内置、CN 版保证 CJK 严格
2× 拉丁步进。

## 变更摘要（Change Summary）

- `assets/fonts/MapleMonoNF-CN-subset.ttf`（7.4MB，18780 字形，OFL，
  许可见同目录 MapleMono-OFL.txt）：maple-font v7.9
  MapleMonoNormal-NF-CN-Regular.ttf 经 pyftsubset --no-layout-closure
  --no-hinting 子集化；unicodes = latin/符号区段（20-24F、2000-2BFF 含
  制表符全量、3000-33FF、FE30-FE6F、FF00-FFEF）+ GB2312 汉字（用
  codec 自枚举 bytes([b1,b2]).decode('gb2312')，7446 字符）+ NF PUA
  E000-F8FF + plane 15/16 图标区。全字体 20.6MB -> 7.4MB。
- 接线 `crates/app/src/ui/fonts.rs`：注册 `terminator-mono` 到
  Monospace/Proportional 家族链 **insert(0)**（egui 默认 Ubuntu/Emoji
  回退保留），`terminator-cjk`（Noto 子集）仍在链末兜底 Hangul/
  全角拉丁/①㈱/生僻字（Maple 没有的码位）。`env_font_data(var)`
  泛化出对称的 `TERMINATOR_FONT=path[:face_index]` 主字体 env 覆盖
  （复用既有 parse/校验/降级路径）。
- 宽字收敛：Maple 的 adv(汉)=1.2em 恰好 2× adv(M)=0.6em，
  `measure_cells` 的 scale 收敛到 ~1.0（实测 13.99@14pt），汉英混排
  不再需要 1.204 补偿缩放。grid.rs 测试断言改为 |wide_size-14|<0.2
  （回退到 ~1.2 即 Noto 抢先会 FAIL）。
- e2e 同步：`e2e-cjk.sh` cmap 门改为双字体分治校验（Maple 查 ASCII/
  制表符/GB2312/图标 + 2:1 步进探针；Noto 查全量旧样本含 Hangul），
  像素宽字过滤带 9.5/12.5 -> 8.5/10.5（实测 Han 墨迹 11-13×12-13px，
  ASCII 最宽 8px，分带间隙健康）。

## 验证（Verification）

- cargo fmt / clippy / test --workspace 全绿（app 72 测试）。
- `e2e-cjk.sh` ALL GREEN（cmap、ctl 回返、像素门）；`e2e-ui-style.sh`
  ALL GREEN（G 标题居中 6px 容差不受字体影响）。
- 手动冒烟（Xvfb + ctl send）：powerline e0b0 / code f121 / heart
  f004 图标实心墨迹（interior 49/34/63），ghostty 按 2 格推进 PUA，
  14-16px 图标墨迹填满双格跨距，混排 ASCII/汉字无错位。

## 注意（Notes）

- Hangul 仍走 Noto 兜底（Maple CN 无韩文）；其 1.0em 步进在 ~14pt
  下略窄于双格跨距，字形居中留空，与既有行为一致。
- 连字不生效（网格逐格绘制），现状行为非目标；Bold/Italic 不参与
  egui FontId 渲染，只取 Regular 一档。
- 二进制净增 ~7.4MB；deploy-remote.sh 打包内嵌字体自动携带。
