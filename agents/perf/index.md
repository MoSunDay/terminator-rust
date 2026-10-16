# 性能测量（P0 CPU 优化）

## 目标 / 验收口径（三层）
- **tier1 断言（gate 实跑：`release.yml` → `scripts/deploy/verify-release.sh` → `e2e-idle-cpu.sh`）**
  - S1：`max1s < 1%`（单核）**且 frames == 0** —— 唯一的 `<1%` 硬断言
    （`IDLE_GPU_BACKEND=gl` 软件后端，无宿主驱动地板；见下方 GPU BACKEND 段）。
  - S2 / S13：`fps <= 65`（`TERMINATOR_FRAME_FLOOR_MS` 真节流上限）；CPU 只记录。
  - S7：runaway 兜底断言 `max1s <= 50%`；CPU 只记录。
  - WAKE：`ctl send` 回声 <= 2s 到达 `ctl capture`（无 tick 后唤醒路径的活体证明）。
- **tier2 记录（无阈值）**：矩阵里其余可跑场景（S1b/S3/S4/S8/S11）与真机
  `scripts/deploy/measure-cpu-remote.sh` 的 s1..s13 全部只报数字。真机 idle 地板是宿主
  驱动线程（NVIDIA `[vkps] Update` ~0.3-0.4%/核、**零帧**），native 下 `<1%` 不可达，
  这是环境成本不是应用回归。tier1 的 `<1%` 只在 gl 沙箱成立；`ci.yml` 不跑本 gate。
- **tier3 未实现**：S5/S6/S9/S10/S12（本地 e2e 扩展，见矩阵"判定"）。
- 口径：max1s = 连续整数秒桶内 `cpu秒 / 该桶覆盖墙秒 * 100`，100.0 = 跑满一核；
  avg = 全窗口 cpu秒/墙秒；线程展开 = 同窗口 `/proc/<pid>/task/*/stat` 的 utime+stime 增量。
- 不得写入推断数字；基线表只填真实采样结果。

## 工具
### [cpu-sample.py](../../scripts/bin/cpu-sample.py)
```
python3 scripts/bin/cpu-sample.py --pid PID --seconds N \
    [--interval 0.1] [--json] [--top-threads 5]
```
- 数据源 `/proc/<pid>/stat` 字段 14/15，CLK_TCK 取 `os.sysconf('SC_CLK_TCK')`；线程名取 `comm`（内核截断 15 字符）。
- `--json` 单行对象：`{"pid","seconds","avg_pct","max1s_pct","ticks_total","threads":[{"tid","comm","pct"}]}`；
  note（中途退出/SIGINT）一律走 stderr，stdout 只有一个 JSON 对象。
- 退出码：0 正常（进程中途退出仍是 0 + note）、2 pid 不存在/不可读、130 SIGINT（打印部分结果）。

### [measure-cpu-remote.sh](../../scripts/deploy/measure-cpu-remote.sh)
| 变量 | 默认 | 说明 |
| --- | --- | --- |
| DEPLOY_HOST | 必填（LOCAL=1 时免） | ssh 目标（如 `m@192.168.31.196`） |
| APP | terminator-rust | exe 名或完整路径（多安装时用全路径消歧） |
| CTL | terminator-ctl | 远端 ctl 命令/路径 |
| SCEN | s1 | 逗号分隔场景 id |
| CPU_SAMPLE | scripts/bin/cpu-sample.py | 相对远端仓库根的采样器路径 |
| ROOT | ~/terminator-rust | 远端仓库根 |
| SAMPLE_SECONDS | 10 | 每个场景的采样窗口 |
| SAMPLER_ARGS | 空 | 追加给采样器的参数（如 `--json`） |
| WM_CLASS | terminator-rust | xdotool `--class` 匹配 |
| LOCAL | 空 | 设 1 用本地 bash 跑拼装后的脚本（驱动自测，免 ssh/DEPLOY_HOST） |

- **每个场景一次 ssh 调用**（沙箱/工具环境会跨调用回收进程，setup+采样+teardown 必须在一个远端脚本里）；
  不用任何 `pkill -f`。示例：
  `DEPLOY_HOST=m@box SCEN=s1,s2 SAMPLE_SECONDS=30 SAMPLER_ARGS=--json scripts/deploy/measure-cpu-remote.sh`
- PID 发现：扫描 `/proc/*/exe` 匹配 APP（同名多进程取最新 starttime，多个不同 exe 路径直接拒绝）；
  socket/DISPLAY/XAUTHORITY 从 `/proc/<pid>/environ` 取，**不假设 ipc.sock 位置**；`$CTL instances` 仅作信息性交叉核对
  （主 socket 的 PID 列是 `-`，不可作依据）。
- 远端退出码：0 正常 / 2 环境失败（无 pid、无 pane、无 ctl）/ 3 跳过（缺 xdotool·DISPLAY，或 s7 这类本地专属场景）；
  本地驱动退出码 1 = 有场景失败。`LOCAL=1` 仅用于驱动自测（用本地 bash 代替 ssh）。
- fps 观测：应用 stderr 里的 `frame-stats` 行（见下）；远端或本地日志 `| grep frame-stats`。
- **本脚本只记录、不断言（tier2）**：真机 idle 地板属宿主驱动线程，脚本里没有阈值门；
  阈值 gate 是 `scripts/bin/e2e-idle-cpu.sh`（tier1，见下）。

## 场景矩阵 S1–S13
| id | 场景 | 做法 | 判定 |
| --- | --- | --- | --- |
| S1 | 空转（单窗单 pane） | 远程 s1：直接采样 | 断言 max1s < 1% 且 frames == 0（tier1，唯一 `<1%` 硬门） |
| S2 | 单 pane 60Hz spinner | 远程 s2：`ctl send` 循环 `printf "spin %s\r"` + `sleep 0.016` | 记录（tier2） |
| S3 | S2 + 窗口最大化 | 远程 s3：`xdotool windowsize 100% 100%` + `windowactivate`（WM 相关，best-effort，失败只警告） | 记录（tier2） |
| S4 | 全屏 TUI 风格重绘 | 远程 s4：`ESC[H` + 40 行 `seq`/`sed`，60Hz（payload 用双反斜杠转义，同 e2e-bg-seams） | 记录（tier2） |
| S5 | 4 pane 网格、仅一个 pane 有输出 | 待补：shape_cache 只重绘自身 pane（本地 e2e 扩展；远程脚本 reserve `SCEN=s5`） | 未实现（tier3） |
| S6 | 双 OS 窗口、各一个 pane | 待补：多 viewport 渲染路径（本地 e2e） | 未实现（tier3） |
| S7 | 编辑器打开且无 composition（pane 重命名） | 本地 IME e2e（pane-header 双击几何依赖实时布局，远程不做；`SCEN=s7` 打印 SKIP 并退出 3） | 记录 + runaway 断言 50%（tier1） |
| S8 | 连续鼠标移动 | 远程 s8：`xdotool mousemove_relative --sync 20 0` 循环（每 15 步绝对回位防屏幕边缘卡住；`timeout 1` 兜底） | 记录（tier2） |
| S9 | scrollback 复审（Shift+PgUp/PgDn 静止视图 + 滚动） | 待补：本地 e2e（键注入 + 视口条像素） | 未实现（tier3） |
| S10 | 大范围选择 + Copy | 待补：本地 e2e（选择/ctx.copy_text 路径） | 未实现（tier3） |
| S11 | 边缘 resize 手势 | 远程 s11：右边缘 `mousedown 1`，按住时 +/- 拖动，采样结束 `mouseup 1`（EXIT trap 兜底释放） | 记录（tier2） |
| S12 | tab chip 拖动 / 跨窗拖拽 | 待补：本地 e2e（drag/drop overlay 每帧重绘） | 未实现（tier3） |
| S13 | `cat` 40k 行输出洪泛 | 远程 s13：远端生成 4 万行文件后 `ctl send cat <file>` | 断言 fps <= 65；CPU 仅记录（输出带宽受限） |

## 环境开关
- `TERMINATOR_FRAME_STATS=1`：开启后每秒向 stderr 写一行
  `frame-stats frames=<n> fps=<fps:.1> pass_avg_us=<avg:.0> pass_max_us=<max>`（crates/app/src/render/stats.rs）；
  未开启时零开销。fps 是"根窗口 pass"的频率，含第二窗口在根 pass 内的渲染。
- `TERMINATOR_FRAME_FLOOR_MS`（默认 16）：输出驱动帧的最小间隔（真节流），上限 1000；
  未设置时按 16ms ≈ 60fps。`0` = 关闭节流：不等待，仍是单遍立即唤醒（输出块**不做合并**）。
- 与像素 e2e 的 `TERMINATOR_NO_MOTION`/`TERMINATOR_OPAQUE` 无关，测量场景不要设这两个。

### e2e 采样开关（`scripts/bin/e2e-idle-cpu.sh`）
| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `IDLE_GPU_BACKEND` | `gl` | 整个沙箱 run 用软件后端（llvmpipe；`WGPU_BACKEND` 写进 app 启动 env，S1 之外的阶段也跑它）：宿主 NVIDIA Vulkan 驱动线程 `[vkps] Update` 有 ~0.3-0.4% 单核、**零帧**的常驻地板，会把应用自身成本埋在噪声里；`native` 测真实驱动。判定"我们代码是否为 0"用 `gl`，报告真机成本用 `native` |
| `IDLE_SAMPLE_S` | 8 | S1 窗口秒数 |
| `IDLE_SETTLE_S` | 6 | setup（`ctl list`/`capture`）流量与首帧 GL 光栅抖动之后的静默期；缺它时一个 10ms jiffy 就会顶到 max1s 1.0%。10ms jiffy = 1.0% 正好是门槛：冷启动的 release 包（25MB 二进制 + 17MB 字体页入/解析）实测偶发 1 jiffy，verify-release 首跑红、重跑绿即属此列（2026-10-16 记录）；持续 ≥2 jiffy 才是真回归。S1 现在把首窗抬高时**自动重采一次并取更优窗口**（fps 断言仍是整段单次判定），把偶发 jiffy 的冷启动假阳性概率降下来；两个窗口都 ≥ 门槛才是真回归/真失败 |
| `IDLE_MAX1S_PCT` | 1.0 | S1 门槛（% 单核） |
| `IDLE_MAX_FPS` | 0 | S1 帧数上限（帧必须是 0 才叫空闲） |
| `S7_SAMPLE_S` / `S7_MAX1S_PCT` | 5 / 50 | S7 重命名编辑器窗口与 runaway 兜底 |
| `PIN_SAMPLE_S` | 5 | S1b 常驻探针窗口 |
| `MAX_FPS_SPIN` / `MAX_FPS_BUSY` | 65 / 65 | S2 / S13 帧率上限（配合 floor 16ms ≈ 62.5fps） |
| `APP_BIN` / `CTL_BIN` | target/debug/… | 指向 release 包（`verify-release.sh` 用它） |

- S1/S7 走 `e2e_sandbox` → `TERMINATOR_OPAQUE=1` + `TERMINATOR_NO_MOTION=1`：
  空闲/编辑器场景必须是**帧静止**态；悬停/光标动画由别的像素用例覆盖。
- 软件后端下 S2/S13 的 CPU 数字（llvmpipe 多线程可到 ~600%/核）只记录、
  不断言；真机 `native` 才是可比成本。

## egui 0.36 重绘语义（修正）
- 零延迟 `request_repaint` 只是把 `outstanding` 置 1，同一轮里每个输出事件仍渲染**两遍**（0.36 context.rs）；
  带延迟的请求会先减去 `predicted_dt`，所以旧的 "`request_repaint_after(1ms)` 合并成一帧" 说法是错的：
  1ms 近似立即，仍是单遍。
- 真正的节流是 stats.rs 的 pacing floor：它是 `>= predicted_dt` 的**真实等待**，
  `TERMINATOR_FRAME_FLOOR_MS=16` 才把输出驱动帧钉到 ~60fps。
- 测"无节流上界"时用 `TERMINATOR_FRAME_FLOOR_MS=0`，结果不与 <1% 契约混用；矩阵默认 16。

## 基线记录表
沙箱 `e2e-idle-cpu.sh`（Xvfb 1200x800，`IDLE_GPU_BACKEND=gl`，floor 16ms）：

| 日期 | 构建 | 场景 | max1s% | avg% | fps | 备注 |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-10-16 | P1–P4 全绿跑（run4） | S1 空闲 | 0.0 | 0.0 | 0.0 | frames=0；软件后端下**零 jiffy**，P2 reactive 循环无残余唤醒 |
| 2026-10-16 | 同上 | S1b 常驻探针 | 0.0 | 0.0 | 0.0 | 同上 |
| 2026-10-16 | 同上 | wake 往返（ctl send→capture 标记） | — | — | — | 19ms（门槛 ≤2000ms） |
| 2026-10-16 | 同上 | S7 重命名编辑器（无 composition） | 0.0 | 0.0 | 0.0 | frames=0 + 回车提交成功；P1 闩锁 + caret blink 钉住后成立 |
| 2026-10-16 | 同上 | S2 60Hz spinner | 610 | 605 | 51.7 | CPU 是 llvmpipe 多线程（记录项）；**帧率门 ≤65 通过** |
| 2026-10-16 | 同上 | S13 `cat` 40k 行 | 100.1 | 100.2 | 24.0 | 只记录；其间 `longest_pass=15.85s` = 见下"引擎 fast-fill 停顿" |

引擎 fast-fill 停顿（`seq 1 40000`，native 后端，同一脚本手测）：

| 构建 | 形状 | 结论 |
| --- | --- | --- |
| /tmp/base（2026-10-16 改动前基线） | 连续 16 秒 100% 单核 + RSS 恒定，随后自行恢复 | 与本次改动**无关**；`S13`/`cat` 大输出的 16s 主线程自旋是 vendored ghostty 的既有问题（agent.md "FAST-FILL HANG" 同一族），两个构建逐秒曲线一致 |
| 当前构建（P1–P4） | 同上（16 秒热段，恢复后 1-3%/s，基线 3-7%/s） | 无回归；恢复后残余 CPU 反而更低 |

> 大输出后的稳态残余（1-7%/s）只在 40k 行 scrollback 这种病理态下出现，
> 不在 <1% 契约的场景矩阵里；记录在此以免日后误判。

## 真机落地（2026-10-16，192.168.31.196）
发布 45452f15（pack + `verify-release.sh` 全绿，含 idle 门与打包二进制上的
IME M1–M7），`deploy-remote.sh --no-build --keep-running` + migrate2.sock 分批把
用户 pane 交给新实例（子进程 PID 全部保留），旧实例按 PID SIGTERM。落地后实测：

| 场景 | 旧构建 | 新构建 | 说明 |
| --- | --- | --- | --- |
| 真机空闲（本 pane 静止、focus + motion） | 1.75% avg / 2% max1s | 0.00% 主线程 | 老构建残余主要是 caret blink 重绘；新构建主线程 0.00%，剩余 ~1.5%/s 全在宿主 `[vkps] Update` 驱动线程（**零帧**），`WGPU_BACKEND=gl` 下 0 jiffy |
| 未聚焦窗口静置 | — | ~0.6%/s（驱动） | 与是否出帧无关 |
| 60Hz spinner（date +%N 循环） | 23.2% avg / 26% max1s | 16.7% avg（主线程 15.3%） | 58.5fps，`pass_avg≈0.96ms`（墙钟）/ ~2.6ms CPU/帧 |
| 60Hz spinner，`TERMINATOR_FRAME_FLOOR_MS=0` | — | 单 pane ~57fps | P2 节流上界对照（默认 16ms 即 ~62.5fps 上限） |

- **验收口径**：真机 idle 地板是 NVIDIA 驱动的 present 线程，不是应用代码；
  沙箱 gate（`IDLE_GPU_BACKEND=gl`）里 <1% 断言成立（0.0%、0 帧）。
  真机上与应用帧无关的 ~1.5%/s 应用侧无法消除，报告时请分开归因。
- 60Hz 动画成本 ≈ 帧数 × ~0.25ms CPU/帧；floor 提到 33ms 近似线性减半，
  idle 不受影响。.196 的刷新率未知 → 默认保持 16ms/60fps。
- 运维注记：目标机 `/usr/local/bin/terminator-ctl` 仍是 9 月 6 日旧构建（无
  `instances`），远端一律用 `/opt/terminator-rust/current/bin/terminator-ctl`；
  29015a79 起运行实例曾跑在 `migrate3.sock`（stale 套接字由下次 start 回收）；
  2026-10-17 装入 `current` = 65728655 后当日凌晨已实况切换（见
  agents/verified-end-to-end.md）：运行实例 1052149 = 65728655 镜像 + 默认
  `ipc.sock`，migrate3.sock 随旧实例 SIGTERM 一并删除。

- 交接后一度看到 25fps / 5.2% avg，追查结论**不是回归**：两个用户 ssh agent
  pane 在跑各 ~10Hz 的 spinner（真实活动输出，合并唤醒 25fps），`ctl capture`
  逐 pane 两两 diff 即可定位输出源；`pass_avg≈0.35ms` 说明每帧仍很便宜。
  测真机 idle 前必须先清自己的压测残留循环——本次交接现场就有一个忘停的
  `while :; printf "spin %s\r"` pane 混在采样里。

## 本轮修复后的 release 包 gate（65728655，2026-10-17 凌晨）
`pack-release.sh` + `bash verify-release.sh` **一次全绿**（S1 首窗被偶发 jiffy
抬高时自动重采一次、取更优窗口：降低冷启动假阳性概率；两个窗口都 ≥ 门槛才是
真回归/真失败，重试不吞帧）。同一 release 二进制上 `e2e-idle-cpu.sh` 实测：

| 场景 | 数字 | 判定 |
| --- | --- | --- |
| S1 空转 | avg 0.0% / max1s 0.0% / 帧 0 | tier1 断言通过 |
| S1b 光标钉住 | 0.0% / 0.0% / 帧 0 | tier2 记录 |
| WAKE（`ctl send` 回声） | ~11ms | tier1 通过 |
| S7 重命名编辑器 | 0.0% / 0.0% / 帧 0，提交且无按键泄漏 | tier1 runaway 通过 |
| S2 60Hz spinner | 50.0fps（<= 65） | tier1 断言通过 |
| S13 40k 行 `cat` | 15.6fps、longest_pass 0.00s | tier1 fps 通过；CPU 仅记录 |

- 同源码的 debug 构建这次反而跑出 `longest_pass=16.24s`（release 0.00s），再次
  印证该自旋随构建布局出现/消失（见上文 fast-fill 段），非本轮改动引入。
- 部署：`deploy-remote.sh --no-build --keep-running`，`current` ->
  `releases/65728655…`（prev `29015a79…`，回滚入口由安装器 journal 保证）。
  首次运行步骤 6 冒烟 FAIL：保留实例在 `migrate3.sock`，而 smoke 只认默认
  `ipc.sock`；已修 `stage_smoke`（从 `/proc/<pid>/environ` 读运行实例自己的
  `TERMINATOR_SOCK`），重跑 1–6 步全绿 `PASS (kept-running=1)`。运行实例
  944047 当时仍是 29015a79 的镜像（`--keep-running` 语义）——当日凌晨实况切换至
  65728655（pid 1052149）。
- 冒烟 `ctl list` 只见 pane 2/3：原 pane 5（707327）的 shell 已退出，pane 由
  dead-pane 自动清理移除（pane 2/3 的 zsh 699347/700805 仍存活）。
- 修复内容（e908f61）：`cursor_blink` 改为 fold（后画的非焦点 pane 不再清掉焦点
  pane 的 blink 需求）、`ThemeSyncKey` 改存 palette 值（同名 bg_color/transparency
  变更不再早退）、`bank_snapshot` 抽出 + 新增 pty 测试、gate 加 `app_alive` 探活。

## 后续路线
- P1–P4 已落地并发布（闩锁 / 真节流 / 固定开销 / 脏行快照），数字见上两表。
- 矩阵里仍标"未实现"（tier3）的 S5/S6/S9/S10/S12 留给本地 e2e 扩展，不影响本轮 tier1 契约。
- 大输出 scrollback 后的稳态残余（1–7%/s）与 `seq 1 40000` 的 16s 主线程停顿
  属 vendored ghostty 既有族，单独跟踪，不混入本轮改动。
