Commit: 8ecb8ab47bc992a9522edf0394616e0007744e47

# 输入法

终端组合输入按窗口维护；应用报告输入位置并绘制预编辑文字。原生候选内容由输入法管理；X11 额外修正候选窗对当前输入行的遮挡。

- [input/ime.rs](../../crates/app/src/input/ime.rs)：纯函数计算位置，`sync` 在窗口渲染末尾发布 `PlatformOutput.ime`。
- [input/keyboard.rs](../../crates/app/src/input/keyboard.rs)：接收 `Preedit` / `Commit`，提交通过 `vtask::write` 写入原始 UTF-8，不走按键编码或括号粘贴。
- [render/screen.rs](../../crates/app/src/render/screen.rs)：读取终端帧，用同一位置绘制预编辑并报告候选窗位置。
- [render/preedit.rs](../../crates/app/src/render/preedit.rs)：沿用终端字体、单双宽单元格和 egui 组合下划线；先用终端有效背景色绘制不透明底色，再画文字。底色只覆盖组合区域，受面板裁剪限制，不修改终端缓冲。
- [render/preedit/placement.rs](../../crates/app/src/render/preedit/placement.rs)：纯函数计算下一输入位置及末行滚动预览，原始终端帧保持不变。
- [input/ime/x11.rs](../../crates/app/src/input/ime/x11.rs)：使用 `x11rb` 读取当前焦点和候选窗尺寸；[popup.rs](../../crates/app/src/input/ime/popup.rs) 计算当前显示器内的避让位置。
- [ui/mod.rs](../../crates/app/src/ui/mod.rs)：重命名编辑器通过 `focus_rename_editor` 获取焦点。

## 状态和边界

`WindowUi.ime_anchor` 保存最近一次有效光标。组合期间始终优先取当前光标；隐藏光标或过渡帧才使用记住的位置，空闲且光标隐藏时清空。PTY 提交回显可能晚于下一次 Preedit，不能固定组合开始的位置，否则新预编辑会覆盖刚回显的已提交汉字。

`vt-pane::FrameCursor` 携带真实 `pending_wrap` 和 `autowrap`。启用 DECAWM 时，刚填满一行或剩余列放不下首个宽字符，预编辑从下一行开始。末行只滚动画面副本，为组合文字留出一行；取消后立即恢复原始画面，提交仍由 PTY/VT 执行。DECAWM 关闭时必须忽略 pending wrap，保留终端覆盖末列的行为。

组合期间缺少当前面板时，`effective_target` 使用上次面板与记住的位置。面板会话已退出时关闭 IME；空闲且光标隐藏时同样关闭，此时无法开始组合输入。

`should_interrupt_composition` 始终为 false。不要通过关再开 IME 来取消组合：winit macOS 会清掉客户端 marked text，X11 会重建未重新聚焦的 XIC，均可能破坏后续输入。重命名编辑器自己的 IME 输出优先；交接帧没有编辑器输出时保留终端输出，避免短暂关闭。不要用 `Response::request_focus()` 打断组合。

X11 只使用候选位置矩形的起点，所以报告光标单元格左下角；macOS / Wayland 使用完整光标矩形。平台坐标转换保留在 `anchor_rect`。X11 坐标更新要求 libX11 ≥ 1.8.2。由于 XIM 缺少输入行高度，候选窗向上弹出时可能覆盖当前行；X11 适配层读取实际尺寸，将重叠面板移到当前行下方或上方。仅在应用拥有 X 焦点、窗口 PID 匹配、候选窗可见且符合当前 IBus/Fcitx 的 class 和 override-redirect 属性时处理；候选归属不明确或协议查询失败时跳过。组合期间定时重绘，发现延迟显示的面板；坐标按像素密度转换，并使用当前显示器边界。

## 验证入口

- [状态回归测试](../../crates/app/src/input/ime/tests.rs)：同帧连续组合、新组合定位、临时隐藏光标、即时与延迟回显后的定位、不中断输入。
- `render/preedit.rs` 的绘制测试：底色顺序和透明度、单双宽范围、长文本裁剪、清空后无残留。
- [VT 边界测试](../../crates/vt-pane/src/term/tests.rs) 和 placement 测试：精确行尾、宽字符提前换行、DECAWM 关闭、末行预览与取消；popup 测试覆盖实际高度及负坐标显示器。
- [e2e-ime.sh](../../scripts/bin/e2e-ime.sh)：Xvfb + dbus + ibus/libpinyin 真 XIM 输入，覆盖遮挡与取消恢复的像素检查、提交、重命名及英文切换。使用独立 HOME、D-Bus 和可见的原生面板；M3c 验证延迟回显，M3d 验证隐藏/换行/底部/缩放，M3e 验证精确行尾/宽字符/末行的像素保留、取消和实际提交。`--bundle DIRECTORY` 直接验收发布包二进制。任何失败使整套非零退出；裸 Xvfb 不能验证组合输入。
- [测试索引](../e2e-suites.md)；[已实测能力](../verified-end-to-end.md)。原生窗口实测覆盖 IBus；Fcitx 尚无同等级运行证据。macOS 原生候选窗仍需实机验证，Linux XIM 与交叉编译不能代替。
