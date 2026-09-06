Commit: f0e507fc0c70c1f2b21af97c233ac50c8dd0da84

# 部署脚本 pids_of 的 /proc exe 读取有界重试（K1 同源加固，D1）

## 背景（Context）

`deploy-remote.sh` stage_restart 的 `pids_of()` 以单次
`readlink /proc/<pid>/exe` 过滤 pgrep 命中，与 `e2e-oc-exit.sh` K1
同一竞态类：pid 可在 execve 落地前出现 -> 空读，旧实例被漏出终止
名单，退化为 10s 等待 + SIGKILL（clean TERM 关闭被 flake 掉）。属
评审备案的表外同源加固点（D1），平移 K1 的有界重试方案。

## 变更摘要（Change Summary）

- 空读改为有界重试：20x0.25s，读到非空即 break，之后照旧
  `case "$exe" in "$root"/*)` 过滤；失败路径 5s 上界 fail-close
  （漏杀仍由既有 pid-wait 循环 + SIGKILL 兜底）。
- 与 K1 的 break-to-match **有意不同**：`pgrep -f` 同时命中携带
  本脚本文本的 ssh/zsh wrapper（exe 稳定为 bash、永不匹配
  `$root/*`），break-to-match 会让每次 `pids_of` 调用对每个
  wrapper 白烧 5s（stop 等待循环调用 ~40 次）；只对"空读"重试，
  以同一上界关闭观测到的 flake 类，成功路径零开销。

## 影响面（Impact Surface）

- 仅 `scripts/bin/deploy-remote.sh`（stage_restart 远端 heredoc，
  +11/-1），产品 Rust 代码零触碰。
- 行为套件（从文件原样提取 `pids_of` 验证）：健康实例 28ms 列出
  （零开销）；3 次空读 ~0.8s 恢复并列出；wrapper 类 35ms 排除
  （零开销）；持续空读 ~5.1s 有界排除。

## 语义与边界（Notes / Compatibility）

- 持续空读的 pgrep 命中现实中几乎不存在（cmdline 为空的进程不会
  命中 `pgrep -f`），5s 最坏成本只落在失败路径。
- 该 heredoc 未显式 `set -e`，`[ -n "$exe" ] && break` 在两种
  regime 下都安全（同文件 pid-wait 循环同款 idiom 先例）。
- D2（e2e-oc-exit.sh fail 消息空位）为继承类备案，不在本变更范围。

## 相关文档（Related Docs）

- [../../agents.md](../../agents.md)（deploy 条目：kill by pid 走
  exe 校验，pkill -f 禁用）
