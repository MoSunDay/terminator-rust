Commit: 5f77d36d0be13eedb408c7b1405107f384e87bb5

# 无边框半透明窗口：opacity 设置 / 拖拽 / 单 tab 零 chrome

## 背景（Context）

窗口此前带系统装饰，顶部固定一条 tab 栏；单 pane 场景下这条 chrome 只
为放 tab 名和几个按钮，垂直空间被白占。用户要求无边框、能透出壁纸的半
透明窗口，且窗口仍要可拖动。Xvfb e2e 没有合成器，半透明像素无法混合，
像素断言需要确定性开关。

## 变更摘要（Change Summary）

- 窗口：`NativeOptions` 关装饰（`.with_decorations(false)`）+ 透明
  （`.with_transparent(true)`）；`clear_color` 返回 `TRANSPARENT`，面板
  底色自带 alpha，交由合成器混合。
- `Settings.opacity`（0.5..=1.0，默认 1.0 = 不透明）落 state.json：
  `PSettings` 加 `#[serde(default = "default_opacity")]`，旧文件无该键按
  1.0 加载；非有限值回退 1.0，越界钳到 0.5..=1.0。默认不透明是有意的
  ——裸 X / 无合成器环境里透明像素会发黑，透明是显式选择（Inspector 的
  "Window opacity" 滑杆）。
- `render/colors.rs::with_opacity`：把 alpha 压到 chrome/pane 底色填充
  （`Color32` 预乘存储，亮度随 alpha 变暗）；只作用于"面"，文字/光标/
  选区/焦点 accent 框保持不透明。`grid.rs::DrawArgs.opacity` 由
  `screen.rs` 传入网格底色，滚动条、分隔条、pane header 同样经
  `with_opacity`。
- 拖拽：无系统标题栏后窗口靠内容拖动——tab 栏空白区
  （`tabs.rs::chrome_drag`）与 pane header 条（`pane_header.rs`）注册
  `Sense::drag`，主键按下即发 `ViewportCommand::StartDrag`。两处都先注
  册，其上的 chip/按钮先命中，拖拽落到背景。
- 单 tab 零 chrome：`tabs.len() <= 1` 时 top panel 完全不绘制（pane
  header 顶到 y=0，内容区多出约 31px）；Inspector 入口移入 pane 右键菜
  单；出现第二个 tab 时 chip 行自动回来。控件圆角 5→4、selection 填充
  0.30→0.25（style.rs）。
- e2e：所有 Xvfb 脚本 `export TERMINATOR_OPAQUE=1`（启动时把 opacity
  钉成 1.0，滑杆随后仍可改）；Xvfb display 号从 `:$$` 改随机探测空闲号
  （PID 派生号会撞崩溃残留的 /tmp/.X11-unix socket）；e2e-ui-style.sh 新
  增 I1/I2（单 tab 下 chip 带无 accent/填充像素、header 顶到 y=0）。

## 影响面（Impact Surface）

- state.json 新增 settings.opacity；旧文件按默认 1.0 加载（测试
  `legacy_settings_without_opacity_default`）。
- 默认 1.0（不透明）；有合成器时下调滑杆即半透明。无合成器环境透明像
  素发黑，e2e 脚本统一 `TERMINATOR_OPAQUE=1` 把 opacity 钉成 1.0。
- 单 tab 时 pane 内容区上移约 31px；依赖旧 chrome 偏移的 Xvfb 脚本需同
  步调整扫描区（e2e-cjk.sh 内容区起点）。
- 半透明是整窗统一值；`PaneMeta.transparency` 仍是 pane 底色向主题背景
  的混合，两者独立。

## 备注（Notes）

- `e2e-ui-style.sh` 的期望值由 dracula 常量 + mix() 在脚本内现算：token
  调色只改 `render/colors.rs`，几何改动才需要动脚本（I 检查即新几何）。
- 拖拽手柄先注册是有意的：egui 命中测试取最上层 widget，chip/按钮的点
  击不会被拖拽区吃掉。

## 相关文档（Related Docs）

- `features/changelog/2026-09-06/single-row-chrome.md`（上一轮 chrome 精简）
- `features/changelog/2026-09-10/quit-on-last-close.md`（同批关闭语义）
