Commit: 10c1d1515e5963d9441e8df0f9c0ab3aeecd667c

# Codex 通知校验目标终端

Codex hook 通过父进程找到当前终端。只有继承的 `TMUX_PANE` 指向同一个终端时，才使用 tmux 透传；否则直接写入当前终端，避免终端应用继承旧 tmux 环境变量后把气泡发到别的窗口。终端暂时无法完整接收时会补写或重试，超时则报错。

验证：[单元测试](../../../scripts/bin/test-codex-notice.py)覆盖旧 tmux 环境、当前 tmux 环境、部分写入和忙碌终端；[端到端测试](../../../scripts/bin/e2e-codex-notice.sh)的普通、keeper、脱离终端和 tmux 场景通过。
