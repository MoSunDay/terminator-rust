Commit: 6a2a045b7cd90fb02eb2c8809c1c5214ebe954b3

# 控制通道与 OpenCoder 提交（terminator-ctl / oc submit）

## 背景（Context）

外部 agent 循环此前没有任何可靠方式驱动 terminator-rust 的 pane：VT 屏幕抓取、按键注入、以及向 pane 内运行的 OpenCoder TUI 提交提示词都只能靠模拟敲键盘。调研（/root/opencoder 源码）确认：OpenCoder 的 HTTP `POST /prompt` 会启动 server 自己的 drain，与 TUI runner 并发冲突；而 TUI 自身的 runner 会在 turn 边界（steer）与 idle 边界（queue）活查 `session_inputs` 表并回显，是唯一安全的消费方。

## 变更摘要（Change Summary）

- 新增 UDS 控制通道：app 在 UI 线程 drain 请求（`crates/app/src/ipc/`），
  socket 位于 `$XDG_RUNTIME_DIR/terminator-rust/ipc.sock`（fallback
  `~/.config/terminator-rust/`），pane 子进程通过 `TERMINATOR_SOCK` 继承。
  协议类型在 `crates/ipc-proto`（serde，一行 JSON 请求/响应）。
- 新增 CLI `terminator-ctl`（`crates/ctl`）：`list` / `capture` / `send`（逃生舱）
  与 `oc link|submit|status|sessions|unlink`；pane 以唯一 manual title 或数字 id 寻址，
  pane 重命名现在拒绝重名（名字成为寻址键，sidecar `oc-links.json` 按名键控）。
- 新增 `crates/oc-store`：直接 rusqlite 访问 OpenCoder per-workdir store；
  schema 守卫（`PRAGMA user_version=18` + 列探测，不匹配即拒绝写）；
  `insert_input` 在 `BEGIN IMMEDIATE` 事务内 `admitted_seq=MAX+1` 插入
  `session_inputs`（delivery=steer|queue）；`pending`/`receipts` 只读查询；
  dev 二进制 `oc-store-fixture` 造夹具库并模拟 TUI claim。
- db 定位不复制 workdir 哈希算法：`oc link` 走 pane pid → /proc 子孙树 →
  opencoder 进程 → `/proc/<pid>/fd` readlink 找到它正打开的 `opencoder.db`。

## 影响面（Impact Surface）

- 新 crate：ipc-proto、ctl（bin `terminator-ctl`）、oc-store；workspace 成员 +3。
- app：`main.rs` 持有 `ipc::Ipc`（best-effort，失败仅禁用控制面）；`ui/pane_header.rs`
  重名拒绝；`state.rs` 增加 `manual_title_taken`。
- e2e：`scripts/bin/e2e-ipc-oc.sh`（Xvfb + 假 opencoder 进程）全链路验证。

## 语义与边界（Notes / Compatibility）

- 外部 steer 是边界注入：当前 in-flight LLM 调用完成后于下一 turn 注入；
  硬打断只能由持有 cancel token 的 TUI 进程触发。
- TUI 空闲时提交会 pending 搁浅，直到 TUI 下一次本地交互；`--wait` 超时如实报错
  （exit 1），不假装成功。上游 idle_rekick 轮询补丁未合入前如此。
- SIGTERM 不运行析构，socket 文件会残留：下次 start() 探测后回收（e2e 已覆盖）。
- OpenCoder schema 演进由 user_version=18 守卫兜底，拒绝静默降级。

## 相关文档（Related Docs）

- [../../agents.md](../../agents.md)（repo 根，逻辑图与 hard-won facts）
