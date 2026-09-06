Commit: 2d4226718661d070075fa8de838b2d3350024b8f

# kanagawa-wave 内置主题并置为默认（淡雅 / 低饱和观感）

## 背景（Context）

用户反馈默认配色偏丑，希望换成淡雅、有高级感的颜色，参考
/data00/opencoder 的观感。opencoder TUI 全部走 ANSI 语义槽、无字面
RGB 可抄，故具化为：低饱和 ANSI 渲染质感的内置主题移植（选
kanagawa-wave），并置为注册表默认；旧主题全部保留。

## 变更摘要（Change Summary）

- `crates/theme/src/builtin.rs` 新增 `kanagawa_wave()` 构造器，取
  canonical ghostty/kitty 移植值（bg sumiInk0 `#1f1f28`、fg fujiWhite
  `#dcdcdc`，16 个 ANSI 槽全为哑光粉彩系）。
- `BUILTIN_NAMES` 首位插入 `"kanagawa-wave"` —— 默认切换路径
  （`state.rs` fresh_state、`colors.rs` palette_of 兜底与测试期望）
  三处均取 `BUILTIN_NAMES.first()`，默认整体平移，无散落硬编码。
- `builtin_by_name` 增加大小写不敏感 match 臂。
- 测试同步：注册数 5→6、canonical hex 断言（bg `#1f1f28`、
  normal[6] `#6a9589`）；`registry_has_five_entries` 更名
  `registry_lists_all_builtins`，头注释 "Five palettes"→"Builtin
  palettes"（计数不再写死在名字里）。

## 影响面（Impact Surface）

- chrome 派生色（chrome_bg/hairline/divider/tab_active 等）全部经
  `render/colors.rs` mix() 从 palette 线性推导，随新默认自动平移，
  无需改 token。
- inspector 主题菜单遍历 `builtin_names()`，新条目自动出现。
- e2e 脚本（mouse-key / ipc-oc / oc-exit 预置 catppuccin-mocha、
  ui-style 预置 dracula）均显式指定主题，不依赖默认值。
- remote/zellij bootstrap 收 9-slot palette_hex 参数，无主题名硬编码。

## 语义与边界（Notes / Compatibility）

- 旧主题全部保留，仅默认顺序变化；未知名回退走 palette_of first
  兜底（行为同类平移）。
- 已持久化的 state.json `theme` 字段优先于新默认 —— 存量机器
  （如部署机 192.168.31.196）需切换一次主题或清空该字段才能看到
  新配色（待用户决策，非代码缺陷）。

## 验证（Evidence）

- `cargo test --workspace` 193/193；`cargo clippy --workspace
  --all-targets -- -D warnings` 全绿；`cargo build --workspace` 通过。
- `scripts/bin/e2e-ui-style.sh` ALL GREEN（A–H）。
- fresh HOME Xvfb 像素冒烟：无 state.json 启动，pane 主导背景色
  (31,31,40) 精确命中 sumiInk0。

## 相关文档（Related Docs）

- [../../agents.md](../../agents.md)（默认主题可从 builtin.rs 直查，无需记忆条目）
