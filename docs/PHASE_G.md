# 阶段 G 记录：清理前检查提速

日期：2026-09-30　机器：Windows 11 Pro 10.0.26200，16 个逻辑 CPU，普通用户权限
构建目录：`CARGO_TARGET_DIR=C:\Users\king\AppData\Local\DiskWhy-build\target`
测量对象：清理页**默认勾选**（26 组、911 个路径，扫描时 36 GB；枚举后约 39.6 万个文件、6.5 万个子文件夹）

**没有对用户的任何真实文件执行删除 / 移动 / 回收。** C 盘上的所有测量只做了预检（生成计划 + 逐个检查），弹层里只点了 ×。真正的删除只发生在 Rust 测试自己在 `%TEMP%` 里建的文件上。

## 1. 结论

| | 改之前 | 改之后 |
|---|---|---|
| 点「清理」→ 确认弹层出现 | **192.7 秒**（GUI，旧版 DiskWhy.exe，UI Automation 计时；阶段 F 记录 174–181 秒） | 后端 0.41–0.46 秒出估算；UI Automation 量到弹层 0.59 / 0.66 / 0.81 秒，另一次 2.6 秒（那次机器更忙，扫描本身也从 40 秒变成 54 秒） |
| 点「清理」→ 检查全部完成（按钮可用） | 同上（旧版要等检查完才出弹层） | GUI 20.7 秒（后端 `click_to_done`；UI Automation 量到 23.1 秒）；之前几次 25–26 秒 |
| CLI，同一份快照（热缓存） | 167.9–178.1 秒 | 21.4 秒 |
| CLI，刚扫完盘马上做 | — | 25.5 秒 |

检查结果前后一致：会删约 39.56 万个 / 34.9–35.1 GB；跳过里 被占用 248–252、没权限 57、云端 1 两边都一样（刚改过的数量随时间变）。

## 2. 慢在哪里（`diskwhy-cli preflight --from <快照> --profile` 实测）

改之前 178 秒 = build_plan 55 秒 + preflight 123 秒。拆开单独重跑（热缓存，单线程）：

| 环节 | 用时 | 每个文件 |
|---|---:|---:|
| 只列目录（FindFirstFileExW，6.5 万个） | 7.6–8.6 秒 | — |
| 取指纹（打开 + 文件 ID / 大小 / 修改时间） | 41–45 秒 | 104–114 µs |
| 保护路径逐条检查 | 40–44 秒 | 100–110 µs |
| 　其中纯字符串规则 | 0.6 秒 | 1.7 µs |
| 　其中每条解析父目录真实位置（GetFinalPathNameByHandleW） | 37–41 秒 | 93–103 µs |
| 占用探测（DELETE 打开） | 44–48 秒 | 111–122 µs |
| Restart Manager：被占用的 257–259 个文件一组 | **24.8–26.5 秒** | — |
| 热文件判断 | ≈0（只比较修改时间） | — |

Restart Manager 再细测：逐个问一个文件每次 180–240 毫秒（每次都查到 1 个进程）；1000 个**没被占用**的文件一组只要 1.0–3.7 秒。所以 RmGetList 的耗时基本和“这一组查到几个进程”成正比。按清单项分组、依次问 25 秒；按清单项分组、4 / 8 个会话并发 13 秒；每组 ≤16 个、8 个会话并发 6.7 秒。

并行打开的上限：16 线程下三种打开方式都是每个文件约 35 µs（只读属性 35.4、只 DELETE 37.0、一次打开探测 + 指纹 35.6），4 线程 38 µs、32 线程也还是 31 µs 左右——这台机器上文件打开有一个全局的吞吐上限（和访问方式无关），39.6 万个文件至少要 12–15 秒。试过用父目录句柄相对打开（NtOpenFile + RootDirectory），单线程 56.5 µs、16 线程 37.6 µs，没有更快，没有采用。

## 3. 改了什么

**后端（`crates/diskwhy-core/src/action/`）**
- `plan.rs`：`build_plan` 拆成 `build_plan_quick`（去重合并、重新解释、保护路径 / 节点 / 档位 / 规则一致性 过闸，本机 911 项 0.1–0.4 秒）+ `enumerate_plan`（所有清单项的目录按层用 rayon 并行列，**不打开文件**，3 秒）。`build_plan` 仍是两者合起来，原有调用不变。
- `preflight.rs`：
  - `quote(plan)`：弹层先用的估算（每项按扫描时的大小，被拦下的项、空间提示、隔离区检查照常给出），`pending = true`、`can_execute = false`。
  - `preflight_with(plan, protector, opts, progress)`：每个文件 / 链接**只打开一次**（`winops::probe_and_fingerprint`，`DELETE | FILE_READ_ATTRIBUTES` + 读写删全共享 + `FILE_FLAG_OPEN_REPARSE_POINT`），同时得到占用探测结果和指纹；16 个线程按顺序分块取活，小的清单项先查（被占用的文件都在小项里，Restart Manager 能在大项探测期间做完）。
  - 被占用的文件边探测边交给 Restart Manager：同一清单项的每 16 个一组，最多 6 个会话并发（文档上限每用户会话 64 个）；超过 2048 个被占用文件的部分只标“被占用”不查进程（防极端情况）。`RmStartSession` 返回 `ERROR_MAX_SESSIONS_REACHED`(353) 时稍等重试。
  - 可取消（`Progress.cancel`）：取消后 `plan.preflight_done` 保持 false，执行会被拒绝。
  - 报告里加 `pending`、`timing_ms`（各段用时）。
- `protect.rs`：`check_with` + `ParentCache`。父目录真实位置每个文件夹只算一次；清单项下面的文件夹用“清单项真实位置 + 相对路径”推出（枚举只进入非 reparse 的普通文件夹，所以推出来的和实际解析一致，测试里对比过）。执行时的重新校验仍然逐条、不走缓存完整检查。
- `winops.rs`：`probe_and_fingerprint`；`fingerprint` 拆出 `fingerprint_of`（普通文件不再多问一次 FileCompressionInfo，只有压缩 / 稀疏 / reparse 文件才问）。
- `mod.rs`：`Progress`（原子计数，界面层自己定时读）、`ProgressView`、`PlanItem.cluster`。

**Tauri（`src-tauri/src/lib.rs`）**：`preview_plan(items, strategy, onEvent)` 先发 `quote`，再在后台枚举 + 检查，每 150 毫秒发一次 `progress`，做完才返回完整报告并保存计划；新的一次预览或 `cancel_preview` 让旧的尽快停下（返回“已取消”，计划不保存）。设了 `DISKWHY_PREFLIGHT_LOG=<文件>` 时把各段用时追加一行 JSON（排查用）。

**前端**：`store.doPreview` 收到 `quote` 就打开弹层；弹层里 `pending` 时大数字变淡、「跳过」一行换成“正在列出文件 N / 正在检查 N / M / 正在查占用的程序 N / M”+ 细进度条，主按钮“正在检查…”且不可用；做完换成完整报告（同一个 plan_id，勾选状态保留）。关掉弹层会取消后台检查。`labels.checkLine` 负责进度文字。

**CLI**：新子命令 `diskwhy-cli preflight <盘>|--from <快照> [--profile]`，按清理页默认勾选做快速计划 → 枚举 → 检查并计时；`--profile` 分环节单独重跑（上面第 2 节的数字）。只读。

## 4. 9 条护栏还在哪里执行

| # | 护栏 | 位置 |
|---|---|---|
| 1 | 档位门槛（keep / 未知 / AI 推测 / 只能用工具 拒绝） | `plan::build_item` → `preflight::tier_gate`（快速计划里，没变） |
| 2 | 保护路径 | 整项：`build_item` 最先查、`preflight_with` 再查一次；条目：`decide` 里 `entry_blocked_cached`（逻辑同 `check`，父目录走缓存）；执行：`execute::run_item` 整项再查、`reverify` 逐条不走缓存完整查 |
| 3 | 规则一致性 | `build_item` → `rule_consistent`（没变） |
| 4 | 30 分钟内改过的跳过 | `decide`（用这次打开取到的指纹里的修改时间）；执行时 `reverify` 再看一次 |
| 5 | 云占位 / 云文件跳过、不打开 | 枚举只看目录项属性；`decide` 在打开之前按属性跳过（测试断言 `fp.is_none()`） |
| 6 | reparse point 不跟随，只删链接自身 | 枚举只进入普通文件夹、链接作为条目；`decide` 打开的是链接本身（OPEN_REPARSE_POINT）；执行 `do_link` 没变 |
| 7 | 占用：Restart Manager 只报告不关进程 | `decide` 一次打开探测 → `rm_coordinator` 分组并发问 `winops::rm_locking_processes`（只调 RmStartSession / RmRegisterResources / RmGetList / RmEndSession，没有 RmShutdown）；执行时 `skip_locked = false` 且有被占用的就整个不做（没变） |
| 8 | 回收站所在卷 < 10% 空间提示 | `report_impl`（估算和完整报告都会给） |
| 9 | 执行前重新校验文件 ID / 大小 / 修改时间 | `execute::reverify`（**没改**；对比的是预检时这次打开取到的指纹） |

另外：`execute` 仍要求 `plan.preflight_done`，而只有检查完整做完才会置位；估算（`pending`）的 `can_execute` 永远是 false；后端只在检查做完后才保存计划。

## 5. 测试

```
cargo test --workspace -- --test-threads=1（DISKWHY_TEST_XVOL_DIR=D:\desk\claudex\13\dw-xvol-test，测后删掉）
  diskwhy-core 单元 16 · action_tests 29 · dedup_tests 5（1 ignored）· explain_tests 30 · mock_tests 8 · real_fs_tests 6
  合计 94 passed, 0 failed, 1 ignored
npm test      3 个文件、26 passed（新增 2 个：预检进度文字、进度条单调）
npm run build ✓（vue-tsc + vite）
npm run tauri build -- --no-bundle ✓
```

新增的 3 个 Rust 测试（`tests/action_tests.rs`，都只用测试自己建的临时文件）：
- `probe_then_restart_manager_flags_only_locked_files`：44 个文件，12 个用 share 0 占住、1 个只给读写共享不给删除共享、1 个给了全部共享；断言一次打开探测对被占用的返回共享冲突且仍能取到指纹、指纹和单独取的一致；整个预检（每组 4 个、3 个会话）后 13 个被占用的都报告了当前进程 PID、其余全部可删且带指纹、至少分了 4 组；之后句柄还能写（进程没被关）。
- `quick_plan_quote_is_pending_and_cancelled_preflight_cannot_execute`：快速计划不枚举、估算 = 扫描大小、不能执行；取消的预检不能执行、文件都在；正常做完和一次性结果一致。
- `parent_cache_derived_paths_match_real_resolution`：经过联接的清单项，推出来的父目录真实位置和 GetFinalPathNameByHandleW 实际解析一致；指向 `C:\Windows` 的联接里的条目用缓存检查照样被拦。

注：最后一次全量测试之后又删掉了一个没采用的实验函数（相对打开）、在 Tauri 层加了用时日志，这两处没有重跑 `cargo test`（C 盘只剩 2.8 GB，没再建 debug 产物）；release 构建通过。

## 6. 截图（`docs/screenshots/phaseG-*`，最终版 exe，真实扫描 C:\，UI Automation 点按钮，PrintWindow 截图）

| 文件 | 内容 |
|---|---|
| `phaseG-1-checking.png` | 点「清理」后约 1 秒：弹层已出现，数字是估算（变淡），「正在列出文件 …」+ 进度条，主按钮“正在检查…”不可用 |
| `phaseG-1b-checking-progress.png` | 检查中：「正在检查 N / 396,xxx」 |
| `phaseG-2-done.png` | 检查完：35.0 GB、「跳过 N 个 正在使用」、主按钮可用 |
| `phaseG-3-skipped.png` | 展开跳过原因和占用的程序 |

截完点 × 关掉，没有执行。

## 7. 没做到 / 没验证的

- 完整检查 20–26 秒，达到“30 秒内”，但离“远低于 30 秒”还有距离：瓶颈是这台机器上每打开一个文件约 35 µs 的全局上限（和访问方式、线程数都无关），39.6 万个文件光探测就要 14–19 秒。每个文件必须真的用 DELETE 打开一次才能知道有没有被占用，没有找到更少打开次数又不降低护栏的办法。
- 弹层出现时间：后端估算 0.41–0.46 秒，UI Automation 量到 0.59–0.81 秒；有一次量到 2.6 秒（那次整机更忙），没有深究是 WebView 渲染还是 UI Automation 自己慢。
- 估算里的数字是扫描时的大小（36.0 GB），检查完换成实际能删的（34.9–35.0 GB），数字会跳一下。
- Restart Manager 超过 2048 个被占用文件的上限分支、会话数满（353）的重试分支没有在测试里触发。
- 真鼠标 / 键盘没有验证（同前几个阶段，模拟鼠标在这台机器上无效）。
