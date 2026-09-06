Commit: 6217df4ec1951214a4b7dae9bbc3ec6860ab7a4a

# 单行 chrome：删除冗余窗口标题行（三层 "shell" 去一层）

## 背景（Context）

chrome 原为两行：title 行（22px，居中绘制活动 tab 名）+ tab chip 行（24px），
再叠加每个 pane 的 header，同一个默认 tab 名 "shell" 纵向出现三次。chip 与
pane header 各有独立职责（切 tab / 控制 pane），居中的窗口标题行纯属第三次
重复，视觉噪音且占用 22px 终端内容区。

## 变更摘要（Change Summary）

- 删除 `tabs.rs` 的 `title_row` 与 `TITLE_H`；chrome 变为单行 chip 行，
  终端内容区每窗多出 22px。
- zoom / inspector 图标单元从 title 行右缘迁至 chip 行右缘：在闭包内以
  `ui.interact` 锚定固定矩形（`row.right() - 5 - ICON`、间隔 4px），
  不占布局游标，行为与悬停提示不变；`TITLE_ICON_ZONE` 常量随之删除。
- 原 title 行底部的发丝线移到合并后 chrome 行的底部（底部 3px 内缩处），
  保持 chrome 与 pane 区之间的一条安静分隔。

## 影响面（Impact Surface）

- `e2e-ui-style.sh` 几何全部随动：E 扫描带 Y+45..58 → Y+17..29（下划线
  Y+25..27）；F 采样 Y+37 → Y+9；G header 条带 Y+62..78 → Y+34..50；
  H 由"标题光学居中"改为"标题文本缺失"守卫（在尾部按钮右侧与图标单元
  左侧之间的空白 chrome 区扫描，期望 0 命中）——防止标题行回归。
- D/B/A/C 采样点在 pane 深处或纯 chrome 区，语义不变。

## 语义与边界（Notes / Compatibility）

- chip 极多时理论上可延伸到右缘图标之下（与旧 title 行同一外观类取舍，
  行宽 1200px 下不触发）。
- 快捷键、编辑器外点取消、Quit 持久化路径均未触碰。

## 相关文档（Related Docs）

- [../../agents.md](../../agents.md)（repo 根，逻辑图与 hard-won facts）
