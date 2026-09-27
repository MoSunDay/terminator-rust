Commit: 3dbeb893859ae4cfce31d603746569f328be0d47

# 重写当日提交作者为 MoSunDay

## 变更摘要

- 应用户要求，用 `git filter-branch --env-filter` 将 2026-09-27 的 9 个提交的
  author 与 committer 统一改为 `MoSunDay <MoSunDay@users.noreply.github.com>`，
  提交信息与日期保持不变。
- 逐提交比对重写前后的 tree 完全一致，确认内容零变化；原本已是 MoSunDay 的
  4 个提交 SHA 不变，4 个 heyang.amos 提交及其后继重写为新 SHA。
- 经 `git push --force-with-lease` 同步远端（旧远端 HEAD 415d0c9 -> 3dbeb89），
  推送前把 remote 从 HTTPS 恢复为 SSH。

## 影响范围

- 纯 git 历史维护操作，未改动任何代码或构建配置。
- 持有旧历史的本地克隆需执行 `git fetch && git reset --hard origin/main`
  同步，否则会与新历史分叉。
