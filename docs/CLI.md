# diskwhy-cli：给 Agent 用的命令行

`diskwhy-cli.exe` 是 DiskWhy 的命令行前端：控制台程序，不需要 WebView / Tauri，规则库已内置在 exe 里。
扫描、清理走的是和桌面版**同一条 core 路径**（同样的保护路径、档位闸、规则一致性、预检、占用 / 正在运行检查、动作日志），
所以 Claude Code、Codex 这类 Agent 可以直接调用它来查看和清理 C 盘。

构建：`cargo build -p diskwhy-cli --release` → `target\release\diskwhy-cli.exe`。不需要管理员。

## 三个命令

| 命令 | 做什么 |
|---|---|
| `diskwhy-cli scan [--drive C: \| --path DIR] [--json]` | 扫描（只读），列出清理候选：稳定 id、档位（safe / review）、可清大小、路径、是什么、删了丢什么、怎么回来。结果缓存成快照 `<数据目录>\cli\scan-C.dwsnap`。 |
| `diskwhy-cli clean (--safe \| --ids a,b) [--to delete\|recycle\|stash] [--stash-dir DIR] [--yes] [--json]` | 不带 `--yes` = **预演**：生成计划 + 预检，列出会删什么、多少字节、哪些会跳过及原因，什么都不动。带 `--yes` 才真的执行，报告实际腾出的空间和跳过项。默认用 60 分钟内的快照（`--max-age` 改，`--rescan` 强制重扫），没有就自己扫。 |
| `diskwhy-cli explain PATH [--json]` | 规则怎么说这一个路径：档位、原因、CLI 能不能清、属于哪个候选 id。 |

通用：`--json` 时 stdout 只有一个 UTF-8 JSON 文档（没有 BOM），进度和提示都在 stderr；`--quiet` 不输出进度。
旧的开发命令（扫描报告 / 覆盖率 / 预检计时）在 `diskwhy-cli dev scan|explain|preflight …`。

退出码：`0` 成功；`1` 用法错误；`2` 什么都没做 / 被拒绝；`3` 部分完成（有跳过、失败或被拒绝的 id）。
真实清理几乎总有被占用 / 刚改过的文件被跳过，所以 `--yes` 正常结束常常是 `3`，看 JSON 里的 `result` 即可。

## 例子

```bat
diskwhy-cli scan --json > scan.json
diskwhy-cli clean --safe                         :: 预演：所有 safe 档（桌面版默认勾选的那些）
diskwhy-cli clean --safe --yes                   :: 真的清：safe 档直接删除
diskwhy-cli clean --ids huggingface-hub          :: 预演 review 档：默认进回收站
diskwhy-cli clean --ids huggingface-hub --to stash --yes   :: 进另一个盘的 DiskWhy暂存区（保留 7 天，可在桌面版还原）
diskwhy-cli explain "C:\Users\me\.cargo\registry\cache"
```

PowerShell 5.1 的 `>` 重定向会给文件加 BOM、可能按控制台代码页转码；需要文件时用 `cmd /c "diskwhy-cli scan --json > scan.json"`，或直接读进程的 stdout。

## JSON 形状（摘要）

`scan --json`（`schema = "diskwhy.scan/1"`）：

```jsonc
{
  "ok": true, "exit_code": 0, "root": "C:\\",
  "drive": { "root": "C:\\", "total_bytes": 213710270464, "free_bytes": 2124472320, "used_bytes": 211585798144 },
  "duration_ms": 56607, "files": 1124151, "dirs": 208640, "elevated": false,
  "totals": { "safe":   { "groups": 24, "bytes": 7742038016,  "cleanable_bytes": 7632285696 },
              "review": { "groups": 41, "bytes": 52458676224, "cleanable_bytes": 12503388160 },
              "keep_bytes": 79805382656, "uncovered_bytes": 34294841344 },
  "candidates": [ {
      "id": "wechat-update", "label": "微信更新包", "tier": "safe", "owner": "微信", "category": "installer",
      "bytes": 1130106880, "cleanable_bytes": 1130106880, "cleanable": true,
      "default_selected": true, "allowed_to": ["delete", "recycle", "stash"],
      "what": "微信自动更新下载的安装包", "lose": "不丢数据；如果还没升级，下次会重新下载。",
      "lose_kind": "redownload", "lose_label": "要重新下载", "comes_back": "用到时会重新下载（可能很大、很慢）",
      "how": ["delete"], "how_detail": "退出微信后删除。", "rule_id": "wechat-update", "heuristic": null,
      "paths": [ { "path": "C:\\Users\\me\\AppData\\Roaming\\Tencent\\xwechat\\update", "bytes": 1130106880, "deletable": true } ]
  } ],
  "snapshot": "…\\DiskWhy\\cli\\scan-C.dwsnap", "rule_warnings": []
}
```

`clean --json`（`schema = "diskwhy.clean/1"`）：

```jsonc
{
  "ok": true, "exit_code": 0, "mode": "dry_run",          // 或 "executed"
  "scan": { "from": "snapshot", "age_secs": 104 },          // 或 "fresh_scan"
  "strategy": { "to": "default", "safe": "delete", "review": "recycle", "stash_dir": null },
  "selected": ["wps-office-old-versions", "…"], "refused": [ { "id": "chrome-profile", "reason": "…keep-tier…" } ],
  "items": [ { "id": "wechat-logs", "path": "C:\\…\\xwechat\\log", "tier": "safe", "dest": "delete", "blocked": null,
               "will_delete_count": 32, "will_delete_bytes": 707680256,
               "skipped_count": 7, "skipped_bytes": 1234, "skipped_by_reason": { "hot": 4, "locked": 3 },
               "skipped_sample": [ { "path": "…\\mm.mmap3", "code": "locked", "reason": "正在被占用：Weixin（PID 21168）…", "processes": ["Weixin (PID 21168)"] } ] } ],
  "warnings": [ { "code": "locked", "message": "…", "blocking": false } ],
  "totals": { "will_delete_count": 32179, "will_delete_bytes": 6918934528, "skipped_count": 2118, "skipped_bytes": 713797632,
              "skipped_by_reason": { "hot": { "count": 127, "bytes": 308658176 }, "locked": { "count": 1991, "bytes": 405139456 } },
              "by_dest": { "permanent": {…}, "recycle": {…}, "quarantine": {…} } },
  "can_execute": true, "est_secs": 3.8,
  "result": null,          // --yes 之后：{ "freed_bytes", "ok", "ok_bytes", "skipped", "failed", "by_dest", "skipped_by_reason", "log_path", "manifest_path", "items": [...] }
  "next_step": "dry run only, nothing was touched. Re-run the same command with --yes …"
}
```

出错时（含用法错误）：`{ "ok": false, "exit_code": 1, "error": "…" }`。

## 安全规则

- **预演优先**：没有 `--yes` 什么都不动；`--yes` 时重新做一遍完整预检，再按和桌面版一样的执行路径处理。
- **keep 档、未知、AI 推测、受保护路径**（Windows、Program Files、WinSxS、用户文档 / 桌面、DiskWhy 自己的数据……）一律拒绝，带原因。
- **review 档只能用 `--ids` 点名**，`--safe` 永远不包含它；review 档**永远不直接删除**，只进回收站或暂存区（`--to delete` 时直接拒绝）。
  预检时某项从 safe 变成了 review，而你没点名它，这一项会被拦下。
- 默认去向：safe 档直接删除，review 档进回收站；`--to recycle` 全部进回收站；`--to stash` 时 review 档进另一个盘的
  `DiskWhy暂存区`（自动挑源盘和系统盘以外、空间够的固定盘，或用 `--stash-dir` 指定），safe 档仍直接删除（和桌面版一样）。
- **被占用的文件、有程序正从里面运行的文件夹、30 分钟内改过的文件、云文件一律跳过**，DiskWhy 从不关闭任何程序；需要管理员权限的报告为跳过。
- 只能用工具清理的（`how` 里没有 `delete`，如 pnpm store、WinSxS）不会删，返回该用的工具 / 命令。
- 每一条动作写进 `<数据目录>\actions.log`；执行状态写 `last-run.json`（中途被杀掉时桌面版会提示“上次清理中断”）。
- 数据目录：`%LOCALAPPDATA%\DiskWhy`；测试时设环境变量 `DISKWHY_DATA_DIR=<别的文件夹>`，快照缓存、日志全放进去。
- 没有 Ctrl+C 中途停止的处理：真的执行时不要强行结束进程（已删的不会回滚；暂存的可在桌面版还原）。
