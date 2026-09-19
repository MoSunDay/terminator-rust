Commit: a57e1b153dbd549e6ce240ffa798f0d17c2edc76

# 多窗口评审收尾：inspector 窗口归属 / pane 窗口列 / W6 / K1 加固 / CI

## 背景（Context）

多 OS 窗口批次（18d846d + 4a57404）评审结论"就绪"，留了四个 TODO：
inspector 面板从 secondary 打开时画在 root 且字号滑条改 root 的
WindowUi（真实缺陷）；S6 pane 的窗口归属未上报；root 最后 pane 关闭
（有兄弟窗口存活）无 e2e 断言；e2e-windows 未接入 CI、e2e-oc-exit K1
spawn 时序偶发。

## 变更摘要（Change Summary）

- inspector 归属（UX 缺陷修复）：`inspector` 开关从全局 `UiState` 移到
  每窗口 `WindowUi`；`inspector::show(ctx, d, idx)` 带窗口下标，在
  **发起窗口自己的渲染 pass** 里绘制 —— root 在 main.rs root pass，
  secondary 在 `show_viewport_immediate` 回调内（该回调同步跑在该视口
  自己的 pass，`Window::show` 的 layer 落在当前视口）。字号滑条写
  `windows[idx]`，不再错改 root。两窗口同时开 inspector 时用
  `Id::new("inspector").with(win_id)` 盐避免 area 状态串台。切入口
  （chrome i 单元、pane 菜单）改为 toggle `st.win_mut().ui.inspector`。
  像素级验证：secondary 点开后 changed-pixels bbox 恰好落在 secondary
  窗口原点偏移处，root 独占区 0 变化。
- S6：`PaneInfo.window`（u64，`#[serde(default)]`，0=旧服务端未知），
  app `list` 用 pane→window 映射填值；ctl 表格加 WIN 列。ipc-proto 单
  测锁旧形状兼容。
- e2e-windows W6：关 ROOT 最后一个 pane（secondary 存活）→ app 存活、
  root X 窗口不动、root respawn 出**新 pane id**、可打字回显、兄弟
  pane 的 marker 不受扰。W5 之前先重新聚焦 secondary。
- e2e-oc-exit K1 加固：发现逻辑收拢为 `oc_pid()`（pane_pid + /proc exe
  双条件整体重试）：修复 agent 间 stale pid 泄漏（旧代码 agent2 失败
  时会拿 agent1 的 pid 通过）、死 pid 全量重发现（不再对已回收 pid
  空转 readlink）、pid 去重断言、放弃时 dump pane "dying words"。
- CI（.github/workflows/ci.yml）：build-test（fmt --check / clippy
  -D warnings / cargo test）+ e2e-windows job；zig 走 PyPI wheel、
  `scripts/fetch-vendor.sh` 采买 ghostty、apt 装 Xvfb+xdotool。仓库顺
  带一次性对齐 rustfmt（HEAD 原本有 24 个文件不 fmt-clean，单独
  chore commit）。

## 影响面（Impact Surface）

- 用户可见：secondary 里开的 inspector 面板就出现在那个窗口里，字号
  也只影响它；`terminator-ctl list` 多一列 WIN。
- ctl 消费方（脚本）若按列数解析文本表格需适配（JSON 消费方向后兼
  容，新增键）。
- CI 首次落地：fmt/clippy 变为强制门。

## 备注（Notes）

- K1 "偶发 exit(1)" 已定位为 **opencode 首跑竞态**，非负载噪声：
  opencode 首次启动会向 `$HOME/.opencoder` 播种（skills 安装器 +
  state 目录），app 一次性并发拉起 3 个 pane，并发首跑互相踩踏——
  裸 pty 复现：新 HOME 上 3 实例 2 死（exit 1），暖 HOME 上 3/3 存活。
  e2e-oc-exit 启动 app 前加 warmup：一次一次性 pty 跑（timeout 15s）
  串行化播种，并以 `install-skills-dep.sh` + `.local/share/opencoder`
  为标记断言播种完成。tui.log 只到 "first frame"，崩溃点在其后、无
  日志（静默 panic）。
- K3 根因是 **opencode 手势又变了**（非负载噪声）：22:41 重建的二
  进制 idle 提示符下 bare Ctrl+D 与单击 Ctrl+C 均**直接退出**
  （status 0，裸 pty master 侧注入实测；Esc 惰性）——早间版本还是
  "双击 ^C 才退出、Ctrl+D 惰性"。K3 改为正向断言：agent2 在 Ctrl+D
  后退出即证明 0x04 穿过了 kitty-flags-7 编码 workaround 送达子进
  程（键被丢则 pane 存活）。K2 双击 ^C 在两种手势下均成立，保持不
  变。教训：写 pty slave（/dev/pts/N）模拟的是终端输出而非输入，
  手势探针必须 master 侧注入（pty.openpty + os.write(master)），
  否则"键到达"无法证明。
- 本机当日外部构建把 loadaverage 打到 90-400+：同负载下隔离复跑
  3 次三 pane 全部稳定存活。
- egui 0.36 事实：`show_viewport_immediate` 回调在子视口自己的 pass
  里执行，期间 `Window::show(ctx,…)` 的 area/content 都落子视口
  （ctx.viewport_id()=子）；嵌入式 fallback（无多视口后端）则全部折
  回单一 OS 窗口，行为可接受。

## 相关文档（Related Docs）

- `features/changelog/2026-09-10/multi-os-windows.md`（被收尾的批次）
