# 阶段 D 验证记录：带护栏的动作

日期：2026-09-23（部分运行跨过了 UTC 零点，日志里的 UTC 时间是 2026-09-24）　机器：Windows 11 Pro 10.0.26200，**普通用户权限（未提权）**
工具链：rustc/cargo 1.93.0，node 22，Tauri 2.11.6，windows-sys 0.61.2，windows 0.61.3（新增，只用于 IFileOperation），sha2 0.10.9（仅测试）
构建目录：`CARGO_TARGET_DIR=C:\Users\king\AppData\Local\DiskWhy-build\target`（和阶段 C 一样，D 盘上没有生成新的 target）。
以下输出都是本机实际运行的原文摘录。

**所有真实的删除 / 移动 / 回收都只针对测试或演示脚本自己在临时目录里创建的文件。** 没有对用户的任何真实文件执行过删除、移动或回收。GUI 演示扫描的是 `scratchpad\dwdemo`（演示脚本建的 68 个假文件），演示结束后连同演示用的规则文件、`actions.log`、`quarantine-roots.json`、D 盘上的测试文件夹一起删掉了。

---

## 1. 做了什么

| 部分 | 位置 | 说明 |
|---|---|---|
| 动作模型 | `crates/diskwhy-core/src/action/mod.rs` | `Plan { items, strategy: RecycleBin \| Quarantine{dest_root}, warnings }`、`PlanItem { path, tier, source, rule_id, lose, entries, decisions, dirs … }`（`entries_preview(n)` / `entry_count()` / `bytes()`）、`Entry`（文件 / 链接 / 云目录 + 指纹）、`Decision`、`SkipReason` |
| 计划 | `action/plan.rs::build_plan` | 前端只传路径（+ 加入时看到的规则 id）。**档位、来源、规则由后端按当前扫描树和规则库重新算**；被拦下的项不枚举；能进的项重新枚举磁盘上的真实内容（扫描树每目录只留 50 个文件，不能当删除清单），每个文件取指纹（卷序列号 + 128 位文件 ID、大小、修改时间、链接数、磁盘占用）；嵌套的清单项合并 |
| 预检 | `action/preflight.rs::preflight` | 9 条护栏，见第 2 节 |
| 保护路径 | `action/protect.rs` | 硬编码、不可配置，见 2.2 |
| 执行 | `action/execute.rs::execute` | 回收站 / 隔离区 / 链接；执行前重新校验；最多 4 个清单项并发；失败不回滚已成功的项；每条写日志 |
| 隔离区 | `action/quarantine.rs` | `<dest_root>\DiskWhy-Quarantine\<UTC 时间戳>-<随机>\<盘符>\<原路径>` + `manifest.json`；`restore` / `purge` / `list_batches`（超过 7 天标 `expired`，**不自动清**） |
| 日志 | `action/log.rs` | `%LOCALAPPDATA%\DiskWhy\actions.log`，JSON lines，13 个固定字段 |
| 指引 | `action/guidance.rs` | builtin_tool / migrate：可复制的命令 + 解释；`ms-settings:storagesense` 等白名单设置页。**不运行任何命令** |
| 刷新 | `action/refresh.rs::refresh_dirs` | 动作后只重扫受影响的子树，替换进扫描树，祖先重新加总，用新的卷空间重算归属 |
| Windows 调用 | `action/winops.rs` | 见第 6 节 |
| Tauri 命令 | `src-tauri/src/lib.rs` | `action_env`、`preview_plan`、`execute_plan`（Channel 进度）、`list_quarantine`、`restore_batch`、`purge_batch`、`get_guidance`、`open_system_settings`、`pick_scan_folder`、`pick_folder`、`startup_scan_dir`（`--scan <文件夹>`） |
| GUI | `src/components/{DetailPanel,CleanupDrawer,PreviewDialog,QuarantineDialog}.vue`、`src/action.ts`、`src/api.ts` | 见第 5 节 |

### 1.1 几个设计决定

- **文件夹本身保留**：清理一个文件夹 = 处理它里面的文件和链接，再把**变空的**子文件夹删掉（`RemoveDirectoryW` 只删空目录）；清单项本身的文件夹保留（比如 `Temp` 自己不会被删）。
- **按条目处理，而不是整个文件夹一把删**：这样热文件、被占用的、云占位、被替换的文件才能单独跳过，而不影响其余的。
- **how 里没有 delete 的 safe / review 项也拒绝**（例如 `conda-pkgs`、`delivery-optimization` 只能用工具清）：给出该用的工具，不让手动删。
- **链接**：永远不跟进去。隔离且同卷时把链接对象本身改名进隔离区（可还原）；其余情况用 `RemoveDirectoryW` / `DeleteFileW` 只删链接本身（链接不占空间，不进回收站；日志和 manifest 里记下，还原时说明“链接无法还原，需要的话自己重建”）。
- **只有“名字代理”类 reparse 才当链接**（`IsReparseTagNameSurrogate`：符号链接、联接、挂载点）；WOF 压缩、重复数据删除等 reparse 文件按普通文件处理。
- **隔离区位置必须能写**：预检时实际试建一次 `<dest>\DiskWhy-Quarantine`（原来没有就马上删掉空文件夹）。演示时发现本机普通用户**不能在 `D:\` 根目录建文件夹**（`icacls D:\` = `BUILTIN\Administrators:(F)`、`Everyone:(RX)`），所以抽屉里加了“其他文件夹…”（可输入 / 浏览），不能写的位置在预览里就被拦下（截图 phaseD-3c）。
- **execute 返回 Err 时一定还没动过任何文件**（没预检、选了不跳过被占用的、隔离区建不了）；这时后端把计划放回去，可以改完再执行。文件动过之后出的问题（例如最后保存 manifest 失败）只写进报告，不返回 Err。

---

## 2. 护栏与对应的测试

测试文件：`crates/diskwhy-core/tests/action_tests.rs`（26 个，真实临时目录 + 真实 Win32 API），另有 `action::protect::tests::normalize_variants`、`action::tests::stamps` 两个单元测试，以及前端 `src/action.test.ts`（4 个）。

### 2.1 总表

| # | 护栏 | 实现 | 测试 | 结果 |
|---|---|---|---|---|
| 1 | 档位门槛：只有 safe / review；keep、Unknown、Llm 拒绝（另：how 里没有 delete 的拒绝） | `preflight::tier_gate`，`build_plan` 里调用 | `tier_gate_rejects_keep_unknown_llm_and_tool_only` | ok |
| 2 | 保护路径硬编码（本身 / 里面 / 祖先；大小写、`\\?\`、`/`、结尾的点、`..`、UNC、NTFS 流、短文件名、链接绕路） | `protect::Protector` | `protect_rejects_volume_roots`、`protect_rejects_windows_winsxs_installer`、`protect_rejects_program_files`、`protect_rejects_programdata_microsoft_windows`、`protect_rejects_personal_folders_and_direct_children`、`protect_rejects_secret_dirs`、`protect_rejects_onedrive_root`、`protect_rejects_recycle_bin_and_svi`、`protect_rejects_ancestors`、`protect_rejects_bad_path_forms`、`protect_allows_ordinary_cache_paths`、`protect_resolves_links_and_short_names_on_real_fs`、`normalize_variants` | 全部 ok |
| 3 | 规则一致性：路径必须被该规则的 match 命中（或在 tree 规则命中的文件夹里面）；不许删规则目录的父目录；加入时和现在的规则必须一致 | `preflight::rule_consistent` | `rule_consistency_rejects_parent_of_rule_dir_and_changed_rule` | ok |
| 4 | 热文件：30 分钟内修改的跳过并计数、列出 | `preflight::decide` + 执行时再看一次 | `hot_and_cloud_skipped_and_junction_removed_without_touching_target`（hot.bin）、`action_log_one_line_per_action_with_all_fields`（跳过也记日志） | ok |
| 5 | 云占位：跳过并说明，**不打开**（没有指纹） | 同上 | `hot_and_cloud_skipped_…`（`attrib +O`，断言 `fp.is_none()`） | ok |
| 6 | reparse point：不跟随，只删链接自身，预览里标出 | `plan::enumerate` / `execute::do_link` | `hot_and_cloud_skipped_and_junction_removed_without_touching_target`（`mklink /J`，删后目标 3 个文件 SHA-256 不变）、`protect_resolves_links_…`（联接指向 `C:\Windows` 的路径被拒） | ok |
| 7 | 占用检测：Restart Manager 报告进程名 + PID，只报告不杀；可“跳过被占用的”或“取消” | `winops::rm_locking_processes`（只调 RmStartSession / RmRegisterResources / RmGetList / RmEndSession，从不调 RmShutdown） | `locked_file_reported_by_restart_manager_and_process_not_killed` | ok |
| 8 | 空间提示：回收站策略且源所在卷可用 < 10% → 警告 + 切换到隔离区的入口 | `preflight::report` | `recycle_low_space_warning_matches_volume_state` | ok |
| 9 | 重新校验：执行前再取文件 ID / 大小 / 修改时间，对不上跳过 | `execute::reverify` | `reverify_skips_files_replaced_or_modified_after_plan` | ok |
| — | 回收站真的收到了 | `winops::recycle`（IFileOperation） | `recycle_bin_really_receives_file_then_restore_only_ours` | ok |
| — | 隔离区 round-trip（同卷 / 跨卷） | `quarantine::move_file` / `restore` | `quarantine_round_trip_same_volume`、`quarantine_round_trip_cross_volume` | ok |
| — | 隔离区彻底清空只删那一批 | `quarantine::purge` | `quarantine_purge_removes_only_that_batch` | ok |
| — | 隔离区位置不能写 → 预检拦下、执行也不动任何文件 | `preflight::quarantine_dest_problem` | `quarantine_unwritable_destination_is_blocked_before_anything_moves` | ok |
| — | 日志每个动作一行、字段齐全 | `log::ActionLog` | `action_log_one_line_per_action_with_all_fields` | ok |
| — | 动作后只重扫受影响的子树（含还原后重新建出的子目录） | `refresh::refresh_dirs` | `refresh_rescans_only_affected_subtree` | ok |
| — | builtin_tool / migrate 只给命令不运行；设置页白名单 | `guidance` | `guidance_gives_commands_but_never_runs_them` | ok |

### 2.2 保护路径（每类一个用例，节选断言）

| 类别 | 被拒绝的输入（节选） |
|---|---|
| 卷根 | `C:\`、`c:`、`\\?\C:\`、`d:/`、`E:\`、`C:\.` |
| Windows / WinSxS / Installer | `C:\Windows`、`c:\windows\system32\drivers`、`\\?\C:\WINDOWS\Temp`、`C:/Windows/`、`C:\Windows.\Fonts`、`D:\Windows\x`；`C:\WINDOWS\winsxs\amd64_foo`、`D:\Backup\WinSxS\x`；`C:\Windows\Installer\1a2b.msi`、`\\?\c:\windows\INSTALLER` |
| Program Files* | `C:\Program Files\App\cache`、`C:\PROGRAM FILES (X86)\Foo`、`\\?\D:\Program Files\x`、`E:\Program Files (Arm)` |
| ProgramData\Microsoft\Windows | 本身、`…\Start Menu\Programs`；祖先 `c:\programdata\MICROSOFT`、`C:\ProgramData`（`C:\ProgramData\SomeVendor\Cache`、`C:\ProgramData\Microsoft\Office\cache` 放行） |
| 桌面 / 文档 / 图片 / 视频 / 音乐 / 下载（根 + 直接子项） | `C:\Users\alice\Desktop\todo.txt`、`c:\users\ALICE\documents\Projects`、`C:\Users\bob\Downloads\setup.exe`、`\\?\C:\USERS\BOB\DOWNLOADS`；已知文件夹重定向到 `D:\Docs` 时 `D:\Docs`、`D:\Docs\report`（更深一层 `…\Downloads\proj\node_modules` 按要求放行，由档位 / 规则另外把关） |
| .ssh / .gnupg / .aws | `C:\Users\alice\.ssh\id_ed25519`、`C:\Users\alice\.GnuPG`、`…\AppData\Local\x\.aws\credentials`、`D:\work\.ssh` |
| OneDrive 根 | `C:\Users\alice\OneDrive`、`C:\Users\carol\OneDrive - Contoso`、`c:\users\alice\onedrive\` |
| $Recycle.Bin / System Volume Information | `C:\$RECYCLE.BIN\S-1-5-21-1\$R0ABC.bin`、`\\?\d:\system volume information\x` |
| 祖先 | `C:\Users`、`C:\Users\alice`、`D:\` |
| 路径形式 | `…\AppData\..\Desktop`、`\\server\share\cache`、`\\?\UNC\server\share`、`relative\cache`、`C:\temp\x:stream`、`C:\PROGRA~1\App`（纯字符串模式展不开短名 → 拒绝） |
| 真实文件系统 | 临时目录里 `mklink /J win-link C:\Windows`，`win-link\System32` 被拒（原因里有“实际位置”）；联接自身放行；`C:\PROGRA~1` 展开成 Program Files 后被拒 |

生产环境的 `Protector::from_system()` 另外用当前进程的 `%SystemRoot%`、`%ProgramFiles*%`、`%ProgramData%`、`%USERPROFILE%`、`%OneDrive*%`、`SHGetKnownFolderPath`（6 个个人文件夹的真实位置）和本机所有固定盘的根。整项检查一次、预检再查一次、执行时每项 / 每条再查一次；文件夹里面的每一条也单独查（例如缓存目录里藏着 `.ssh` 会被跳过）。

### 2.3 测试原文（`cargo test --workspace -- --nocapture --test-threads=1`，最后一次全部代码定稿后）

```
     Running unittests src\lib.rs (…\diskwhy_core-aa410554f2276e6c.exe)
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
     Running tests\action_tests.rs (…\action_tests-262154a3563d9ec6.exe)
running 26 tests
test action_log_one_line_per_action_with_all_fields ... 日志 8 行，示例：{"ts":1790219996,"time":"2026-09-24T03:19:56Z","batch":"20260924-031956-a0501","op":"quarantine","path":"C:\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-log-104576-…\\cache\\a.bin","dest":"C:\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-log-q-104576-…\\DiskWhy-Quarantine\\20260…
ok
test guidance_gives_commands_but_never_runs_them ... ok
test hot_and_cloud_skipped_and_junction_removed_without_touching_target ... 预检警告：[
    "C:\\ 只剩 16.97 GB（不到 10%）。回收站就在同一个盘上：删除后空间不会立刻腾出来，要等你清空回收站才行。想马上腾出空间，可以改成“隔离到其他盘”。",
    "有 1 个文件是最近 30 分钟内改过的，可能正在用，已跳过。",
    "有 1 个云端文件 / 占位文件（OneDrive 等），已跳过：删了可能连云端的一起删。",
]
ok
test locked_file_reported_by_restart_manager_and_process_not_killed ... Restart Manager 报告：[LockingProcess { pid: 104576, name: "action_tests-262154a3563d9ec6.exe", service: "" }]
ok
test protect_allows_ordinary_cache_paths ... ok
test protect_rejects_ancestors ... ok
test protect_rejects_bad_path_forms ... ok
test protect_rejects_onedrive_root ... ok
test protect_rejects_personal_folders_and_direct_children ... ok
test protect_rejects_program_files ... ok
test protect_rejects_programdata_microsoft_windows ... ok
test protect_rejects_recycle_bin_and_svi ... ok
test protect_rejects_secret_dirs ... ok
test protect_rejects_volume_roots ... ok
test protect_rejects_windows_winsxs_installer ... ok
test protect_resolves_links_and_short_names_on_real_fs ... ok
test quarantine_purge_removes_only_that_batch ... ok
test quarantine_round_trip_cross_volume ... 跨卷：C:\ → D:\desk\claudex\13\dw-xvol-test
ok
test quarantine_round_trip_same_volume ... ok
test quarantine_unwritable_destination_is_blocked_before_anything_moves ... 预检：在 C:\Users\king\AppData\Local\Temp\diskwhy-act-unwritable-q-104576-… 里建不了隔离区文件夹（拒绝访问。 (os error 5)）。普通用户常常不能在盘的根目录（如 D:\）建文件夹，请在清理清单里选“其他文件夹…”，换成这个盘上你能写的文件夹。
执行：（同上）
ok
test recycle_bin_really_receives_file_then_restore_only_ours ... 回收站记录：C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$I9A7YUA.bin → C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$R9A7YUA.bin（原路径记录为 C:\Users\king\AppData\Local\Temp\diskwhy-act-recycle-104576-1790219998005126100\cache\diskwhy-recycle-probe-104576-1790219998005367500.bin）
ok
test recycle_low_space_warning_matches_volume_state ... C:\ 可用 8.5% → 警告：C:\ 只剩 16.97 GB（不到 10%）。…
ok
test refresh_rescans_only_affected_subtree ... ok
test reverify_skips_files_replaced_or_modified_after_plan ... a: ("skipped", "生成清单之后被改过：文件 ID 变了（被替换成了另一个文件）（防止删掉被替换的文件，跳过）")
b: ("skipped", "生成清单之后被改过：大小从 4096 字节变成了 4100 字节（防止删掉被替换的文件，跳过）")
c: ("skipped", "生成清单之后被改过：修改时间变了（防止删掉被替换的文件，跳过）")
ok
test rule_consistency_rejects_parent_of_rule_dir_and_changed_rule ... parent: 路径「…\parent」和规则「t-nested」的匹配范围对不上，拒绝
inherited: 「…\parent」不在规则「t-nested」命中的文件夹「…\parent\cache」里面，拒绝（不允许删规则目录的父目录）
changed: 加入清单时这项由规则「t-nested」说明，现在变成了「t-cache」，请重新查看后再加入
parent via plan: Some("我还不知道这是什么：先搞清楚它是什么再说，未知的东西一律不能加入清理清单")
ok
test tier_gate_rejects_keep_unknown_llm_and_tool_only ... keep: “建议保留”的东西不能删（测试用个人数据）。想腾空间请看“该怎么腾”：迁移或用系统工具
unknown: 我还不知道这是什么：先搞清楚它是什么再说，未知的东西一律不能加入清理清单
tool: 这一项不建议直接删，应该用系统或软件自带的工具清理。见详情里的“该怎么腾”
llm: 这个说明来自 AI 推测，不能作为删除依据
ok
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.17s
     Running tests\dedup_tests.rs    → test result: ok. 5 passed; 0 failed; 1 ignored
     Running tests\explain_tests.rs  → test result: ok. 22 passed; 0 failed
     Running tests\mock_tests.rs     → test result: ok. 8 passed; 0 failed
     Running tests\real_fs_tests.rs  → test result: ok. 6 passed; 0 failed
（diskwhy / diskwhy_lib / diskwhy_cli 三个 crate 各 0 个测试，均 ok）
```

合计 **83 个 Rust 测试全绿**（阶段 A–C 的 55 个 + 本阶段 28 个），1 个 `#[ignore]` 是阶段 C 的大目录对照。跨卷测试需要另一个盘上**可写**的文件夹：本机 `D:\` 根目录普通用户不能写，所以用 `DISKWHY_TEST_XVOL_DIR=D:\desk\claudex\13\dw-xvol-test`（测试前建、测试后删掉了）；没设这个变量且找不到可写位置时测试打印 `SKIP` 并返回（会在输出里看到，不会悄悄算通过——本次运行是真的跨卷执行了）。

前端 `npx vitest run`：

```
 ✓ src/action.test.ts > 确认框：必须输入合计 GB 数 > 两位小数一致才算确认
 ✓ src/action.test.ts > 加入清理清单的按钮只给 safe / review 且可以直接删的项 > safe / review 可以
 ✓ src/action.test.ts > 加入清理清单的按钮只给 safe / review 且可以直接删的项 > keep、未知、AI 推测、只能用工具的不行
 ✓ src/action.test.ts > 加入清理清单的按钮只给 safe / review 且可以直接删的项 > 系统保留 / 映射 / 无权限 / 其余小文件 / 归属色块 不行
 （src/treemap.test.ts 7 个，同阶段 C）
 Test Files  2 passed (2)
      Tests  11 passed (11)
```

`vue-tsc --noEmit` 通过。注：仓库里出现了一个不是我建的 git worktree `.worktrees/phase-b`，vitest 默认会把里面的测试也跑一遍，所以在 `vite.config.ts` 里加了 `test.include = ["src/**/*.test.ts"]`，没有动那个 worktree。

`cargo build --release -p diskwhy-cli` 通过（42.9 s）；GUI 发布版 `npm run tauri build -- --no-bundle` 通过（`Built application at: C:\Users\king\AppData\Local\DiskWhy-build\target\release\diskwhy.exe`）。第一次 GUI 构建在链接时失败（`link.exe` 退出码 `0xc0000142`，当时 C 盘只剩约 12 GB、另有其他构建在跑），空间变多后原样重跑就成功了，代码没改。

---

## 3. 回收站与隔离区的验证细节

### 3.1 回收站（`recycle_bin_really_receives_file_then_restore_only_ours`）

1. 在 `%TEMP%\diskwhy-act-recycle-…\cache\` 下写一个 123,457 字节、内容唯一的文件 `diskwhy-recycle-probe-<pid>-<纳秒>.bin`，修改时间调到 2 小时前，记下 SHA-256；
2. 用测试规则把 `cache` 定为 safe → `build_plan` → `preflight` → `execute`（`Strategy::RecycleBin`，IFileOperation，标志见第 6 节）；
3. 断言：执行报告 ok=1、failed=0；原位置不存在；
4. **在回收站里找到它**：遍历 `C:\$Recycle.Bin\<SID>\$I*`，按 $I 文件格式（版本 2：偏移 24 处 4 字节字符数 + UTF-16 原路径）解析原路径，找到以这个唯一文件名结尾的一条：`$I9A7YUA.bin`，原路径记录为 `C:\Users\king\AppData\Local\Temp\diskwhy-act-recycle-…\cache\diskwhy-recycle-probe-….bin`；对应的 `$R9A7YUA.bin` 存在，大小 = 123,457，SHA-256 与原文件一致；
5. **只动这一条**：把 `$R9A7YUA.bin` 改名回原位置、删掉 `$I9A7YUA.bin`；断言还原后 SHA-256 一致、回收站里再也找不到这个名字。回收站里其他任何条目都没有碰。

说明：`FOF_WANTNUKEWARNING` 保证万一某项“会被永久删除而不是进回收站”（比如超过回收站容量），系统会弹警告而不是静默永久删除；这一分支没有在测试里触发（临时文件很小）。

### 3.2 隔离区

**同卷**（`quarantine_round_trip_same_volume`）：4 个文件（含中文名、三层子目录，1 KB–210 KB）→ 隔离到 `%TEMP%\diskwhy-act-qdest-…`（同在 C 盘）。manifest 每条 `method = "rename"`（`MoveFileExW` 改名）、`source = "t-cache"`、`status = "stored"`、`size` 等于隔离后文件的大小、`quarantined_at` / `mtime` 都有值；隔离后文件的 SHA-256 与原文件一致；原位置没有了、清单项文件夹 `cache` 保留、变空的 `cache\sub` 被删；`list_batches` 列出 1 批、4 条、未过期。`restore` → 还原 4 条（失败 0、跳过 0），批次目录被删；4 个文件回到原位，**SHA-256 和修改时间都与原来一致**。manifest 原文示例：

```json
{
  "original": "C:\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-qrt-96376-1790189559787743800\\cache\\one.bin",
  "stored": "C:\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-qdest-96376-1790189559823636000\\DiskWhy-Quarantine\\20260923-185239-3ecb09\\C\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-qrt-96376-1790189559787743800\\cache\\one.bin",
  "kind": "file", "size": 1000, "bytes": 4096, "mtime": 1790182359, "quarantined_at": 1790189559,
  "item": "C:\\Users\\king\\AppData\\Local\\Temp\\diskwhy-act-qrt-96376-1790189559787743800\\cache",
  "source": "t-cache", "tier": "safe", "method": "rename", "status": "stored", "note": ""
}
```

**跨卷 C → D**（`quarantine_round_trip_cross_volume`）：同样 4 个文件，隔离到 `D:\desk\claudex\13\dw-xvol-test\diskwhy-act-qdest-…`。`MoveFileExW`（不带 `MOVEFILE_COPY_ALLOWED`）返回 `ERROR_NOT_SAME_DEVICE` → 改走“`CopyFileExW` 复制 → 两边逐字节比对 → `DeleteFileW` 删原文件”；manifest 每条 `method = "copy_verify_delete"`，D 盘上的副本 SHA-256 一致；`restore` 反方向（D → C）同样复制 → 校验 → 删除，回到原位后 SHA-256 和修改时间一致。复制后比对不一致或删不掉原文件时，会删掉副本、原文件不动、记为失败（代码路径有，测试里没有人为制造这种故障）。

**彻底清空**（`quarantine_purge_removes_only_that_batch`）：隔离 2 个文件后，把 manifest 复制到隔离区根目录伪造一份 → `purge` 拒绝（“不在 DiskWhy-Quarantine 文件夹里，拒绝清空”）；对真正的 manifest `purge` → 批次目录没了，隔离区根目录里预先放的 `unrelated.txt` 和源目录里别的文件都还在。

**不能写的位置**（`quarantine_unwritable_destination_is_blocked_before_anything_moves`）：对临时文件夹 `icacls /deny *S-1-1-0:(AD,WD)` → 预检出现阻止执行的警告、`can_execute = false`；强行调用 `execute` 返回 Err，源文件原封不动（测后 `icacls /remove:d` 恢复）。

**GUI 演示里的真实一次**（演示假文件，见第 5 节）：清单 = `app-cache`（safe）+ `model-weights`（review），隔离到 `D:\desk\claudex\13\dw-xvol-test`（跨卷）。预检：跳过 4 条（热文件 2、云占位 1、被 PowerShell 进程占住 1——Restart Manager 报告 `Windows PowerShell（PID 100784）`，正是演示脚本里开句柄占住 `in-use.lock` 的那个进程）；执行：成功 54 条、46.18 MB，删掉联接 1 个、变空的子文件夹 3 个，失败 0；联接指向的 `shared-assets` 3 个文件原样还在；然后在“隔离区”里“还原整批”：“已还原 53 个文件（46.18 MB）到原来的位置，跳过 1。”（跳过的 1 条就是那个联接：跨盘不能移动链接，当时只删了链接本身，manifest 里 `method = "link_removed"`）。列表和方块图随之刷新回 55.60 MB。

### 3.3 日志（`action_log_one_line_per_action_with_all_fields`）

3 个文件（1 个热文件）→ 隔离 → 还原，日志 8 行 = 隔离 ok 2 + 热文件跳过 1 + 删空文件夹 1 + 执行汇总 1 + 还原 ok 2 + 还原汇总 1；每行都是 13 个字段（`ts time batch op path dest bytes result reason item source tier method`），不多不少。GUI 演示产生的日志节选（原文）：

```
{"ts":1790219787,"time":"2026-09-24T03:16:27Z","batch":"20260924-031627-7301","op":"skip","path":"…\\dwdemo\\app-cache\\in-use.lock","dest":null,"bytes":40960,"result":"skipped","reason":"正在被占用：Windows PowerShell（PID 100784）（DiskWhy 只报告，不会关掉它们）","item":"…\\dwdemo\\app-cache","source":"demo-app-cache","tier":"safe","method":""}
{"ts":1790219787,"time":"2026-09-24T03:16:27Z","batch":"20260924-031627-7301","op":"remove_link","path":"…\\dwdemo\\app-cache\\shared-assets-link","dest":null,"bytes":0,"result":"ok","reason":"只删了链接本身（…），它指向的内容没动","item":"…\\dwdemo\\app-cache","source":"demo-app-cache","tier":"safe","method":"remove_directory_link"}
{"ts":1790219787,"time":"2026-09-24T03:16:27Z","batch":"20260924-031627-7301","op":"summary","path":"plan-20260924-031621-b75c00","dest":null,"bytes":48422912,"result":"ok","reason":"成功 54，跳过 4，失败 0","item":"","source":"","tier":"","method":"隔离区（D:\\desk\\claudex\\13\\dw-xvol-test）"}
{"ts":1790219797,"time":"2026-09-24T03:16:37Z","batch":"restore-20260924-031627-7301","op":"summary","path":"D:\\desk\\claudex\\13\\dw-xvol-test\\DiskWhy-Quarantine\\20260924-031627-7301\\manifest.json","dest":null,"bytes":48422912,"result":"ok","reason":"还原 53 个，跳过 1，失败 0","item":"","source":"","tier":"","method":""}
```

（`remove_link` 那行的提示文字在截图之后改成了“链接不占空间，直接删除，不进回收站或隔离区”，更准确；行为没变。）

---

## 4. 占用检测细节

`locked_file_reported_by_restart_manager_and_process_not_killed`：测试进程自己用 `share_mode(0)` 打开一个临时文件并一直拿着句柄。

- 预检里先用 `CreateFileW(DELETE, 共享读写删, FILE_FLAG_OPEN_REPARSE_POINT)` 探测：别的句柄没给 `FILE_SHARE_DELETE` → `ERROR_SHARING_VIOLATION`；对这些文件（≤ 20 个时逐个、否则成组最多 1000 个）调用 Restart Manager。报告原文：`[LockingProcess { pid: 104576, name: "action_tests-262154a3563d9ec6.exe", service: "" }]`，PID 等于 `std::process::id()`；警告里也带着这个 PID；
- 选“取消”（`skip_locked = false`）→ `execute` 返回“有 1 个文件被占用，你选择了不跳过，所以什么都没做”，旁边的文件没动；
- 选“跳过被占用的”→ 另一个文件照常隔离，被占用的文件还在；
- 之后测试进程还能往那个句柄里写数据——进程没有被关掉。代码里没有任何 `RmShutdown` / `TerminateProcess` 调用。

---

## 5. GUI（`diskwhy.exe --scan <scratchpad\dwdemo>`，release 版，普通权限，1600×1000 物理像素 / 125% 缩放）

演示数据：`make_demo.ps1` 在 `scratchpad\dwdemo` 下建 68 个假文件（随机内容）：`app-cache`（40 个缓存文件 + GPUCache + 日志 + 2 个刚写的热文件 + 1 个 `attrib +O` 的“云占位” + 1 个被另一个 PowerShell 进程占住的文件 + 1 个指向 `shared-assets` 的联接）、`model-weights`（30 MB 假模型）、`chat-files`、`tool-only`、`mystery`。临时目录的规则来自一份演示用的**用户规则文件** `%LOCALAPPDATA%\DiskWhy\rules\phaseD-demo.toml`（safe / review / keep / 只能用工具 各一条，只匹配 `dwdemo` 下的这几个文件夹），演示后删除。操作方法同前几个阶段：UI Automation 的 Invoke / Value / SelectionItem / ExpandCollapse / ScrollItem 模式点按钮、填输入框，`PrintWindow` 截图。

| 文件 | 内容 |
|---|---|
| `phaseD-1-add-to-cart.png` | 选中 `app-cache`（可以放心清理）：详情面板下方“＋ 加入清理清单”和说明“加入清单不会马上删除……默认进回收站” |
| `phaseD-2-drawer.png` | 右下角清理清单抽屉：2 项（可清理 app-cache 14.63 MB、需判断 model-weights 32.00 MB）、合计约 46.63 MB、策略“隔离到其他盘”→“其他文件夹…” `D:\desk\claudex\13\dw-xvol-test`、“预览时会实际试一下能不能在那里建文件夹” |
| `phaseD-3-preview.png` | 预览对话框：将处理 54 个、46.18 MB（0.05 GB）、删到隔离区；跳过 4 个（热文件 2 · 云端 1 · 被占用 1）、其中 1 个是链接；三条警告（被占用：Windows PowerShell PID …；30 分钟内改过；云占位）；逐条列出会删的文件（相对路径、大小、时间）；红框“这些删了有代价：model-weights（32.00 MB）：删了下次用要重新下载……” |
| `phaseD-3b-preview-skipped.png` | 滚到跳过列表：`cloud-sync-placeholder.pdf`（云端文件，不碰）、两个热文件（“N 分钟前刚改过”）、`in-use.lock`（“正在被占用：Windows PowerShell（PID …）（DiskWhy 只报告，不会关掉它们）”）；确认框里输入了 0.05，按钮变成可点的“跳过被占用的 1 个，执行” |
| `phaseD-3c-preview-blocked-dest.png` | 隔离区选 `D:\`（根目录）时：红色阻止警告“在 D:\ 里建不了隔离区文件夹（拒绝访问。 (os error 5)）……请选‘其他文件夹…’”，执行按钮不可点。（这张是在最后一次构建之前截的，那次构建只改了刷新和结果排序，这个界面没变） |
| `phaseD-4-result.png` | 执行结果：成功 54 个、腾出 46.18 MB、跳过 4、失败 0；隔离区清单路径、日志路径、“已重新扫描受影响的 2 个文件夹（…ms），列表和方块图已更新”；每项明细里跳过的排在前面并写明原因 |
| `phaseD-5-quarantine.png` | 隔离区页面：一个批次（时间、批次号、第 1 天、46.18 MB、54 个文件、位置、原来在哪两个文件夹）、“还原整批”“彻底清空…” |
| `phaseD-5b-quarantine-purge-confirm.png` | 点“彻底清空…”后的二次确认：“彻底删除后无法恢复（不进回收站）。确定吗？”“确定彻底删除”“算了” |
| `phaseD-5c-quarantine-restored.png` | 点“算了”再“还原整批”：“已还原 53 个文件（46.18 MB）到原来的位置，跳过 1。”隔离区变空；背后的汇总卡回到 可放心清理 17.63 MB / 需要判断 32.00 MB，已扫描 55.60 MB |
| `phaseD-6-keep-guidance.png` | keep 项 `chat-files`：没有加入按钮，红框“这个不能删，想腾空间这样做”+“怎么迁到别的盘”（规则里的迁移说明） |
| `phaseD-7-unknown.png` | 未知项 `mystery`：灰色虚线框“先搞清楚它是什么再说……AI 的回答只是推测，同样不能作为删除依据”，没有加入按钮 |

另外在 GUI 里修了两处阶段 C 留下的、扫描“文件夹”（而不是整个盘）时才出现的问题：方块图根层不再画“未归属 178.5 GB”（那是扫描范围之外的整个盘）；顶栏的文件夹下拉框限宽，只显示最后两级。

---

## 6. 查过的官方文档

- `IFileOperation`（shobjidl_core.h）：**只能在 STA 里用**（“cannot be used for a multithreaded apartment (MTA) situation”）→ 每个工作线程 `CoInitializeEx(COINIT_APARTMENTTHREADED)`；调用顺序 SetOperationFlags → DeleteItem → PerformOperations → GetAnyOperationsAborted。
- `IFileOperation::SetOperationFlags`：`FOF_ALLOWUNDO 0x40`、`FOF_NOCONFIRMATION 0x10`、`FOF_SILENT 0x4`、`FOF_NOERRORUI 0x400`（出错当作“跳过”继续）、`FOF_WANTNUKEWARNING 0x4000`（“Send a warning if a file or folder is being destroyed during a delete operation rather than recycled. This flag partially overrides FOF_NOCONFIRMATION.”）、`FOFX_RECYCLEONDELETE 0x80000`（Win8+）、`FOFX_ADDUNDORECORD 0x20000000`（Win8+）。这些值和 windows 0.61.3 里的常量一致。文档里**没有**给 IFileOperation 的 `FOF_NORECURSEREPARSE`，所以链接不交给 Shell，而是自己用 RemoveDirectoryW / DeleteFileW 处理。
- Restart Manager：`RmStartSession`（session key 缓冲区 `CCH_RM_SESSION_KEY + 1`，dwSessionFlags 必须为 0，每个用户会话最多 64 个会话）、`RmRegisterResources`（“Each call … performs relatively expensive write operations. Do not call this function once per file”→ 先用 DELETE 探测筛出被占用的，再成组注册）、`RmGetList`（缓冲区不够返回 `ERROR_MORE_DATA` 234，按 `pnProcInfoNeeded` 重取）、`RmEndSession`。
- `MoveFileExW`：跨卷必须带 `MOVEFILE_COPY_ALLOWED`，否则失败；目录只能同卷移动；`MOVEFILE_WRITE_THROUGH`。跨卷我没用 COPY_ALLOWED（它在“复制成功但删不掉原文件”时也算成功，会留下两份），而是自己复制 → 比对 → 删除。
- `RemoveDirectoryW`：只删空目录；“when lpPathName refers to a directory junction, RemoveDirectory will remove the specified link regardless of whether the target directory is empty or not”，目标不受影响。
- `GetFinalPathNameByHandleW`：`FILE_NAME_NORMALIZED | VOLUME_NAME_DOS` 返回 `\\?\` 形式；缓冲区不够时返回所需大小（含结尾 0）。
- `GetFileInformationByHandle` / `FILE_ID_INFO`（阶段 C 已查）：卷序列号 + 文件 ID 唯一标识文件，用作指纹。
- `SHGetKnownFolderPath`（FOLDERID_Desktop / Documents / Pictures / Videos / Music / Downloads，返回的字符串用 `CoTaskMemFree` 释放）、`ShellExecuteW`（返回值 > 32 为成功）。
- docs.rs：windows 0.61.3 的 `IFileOperation`、`SHCreateItemFromParsingName`（需要 `Win32_System_Com` 特性）、`FILEOPERATION_FLAGS`；windows-sys 0.61.2 的 `Win32_System_RestartManager`（`RM_PROCESS_INFO` 字段）。
- Tauri v2：Channel（进度）、`tauri-plugin-dialog` 的 `blocking_pick_folder`；没有新增插件和前端权限（设置页由 Rust 侧白名单打开）。

---

## 7. 没做到 / 没验证的

- **GUI 里没有演示回收站策略**：回收站只在 Rust 测试里真实验证过（一个文件，送进去再只还原这一条）。GUI 演示用的是隔离区（这样演示完能完整还原，不在用户的回收站里留东西）。
- **“空间不足 10%”的警告没有出现在 GUI 截图里**：截图时 C 盘可用约 20.5 GB（10.3%），刚好不到阈值；测试运行时（8.5%）的警告文字由 `recycle_low_space_warning_matches_volume_state` 和 `hot_and_cloud_…` 的输出证明。预览里的“改为隔离到 X”按钮（会重新预览成隔离策略，`onRequote`）没有在 GUI 里实际点过、也没有截图。
- **`FOF_WANTNUKEWARNING` 的“会被永久删除”警告框没有触发过**（需要超过回收站容量的文件）；回收站成功与否是按“执行后原路径不存在”判断的，不能区分“进了回收站”和“被系统永久删除”——正常情况下后者会先弹出系统警告框。
- **大目录的性能没有测**：枚举和预检是单线程逐文件取指纹 + DELETE 探测；几十万个文件的清单项会比较慢（上限 50 万条，超过整项拒绝）。回收站按每批 256 个调用一次 PerformOperations。
- **跨卷隔离的“复制后比对不一致 / 删不掉原文件”分支**只有代码路径，没有人为制造故障来测。
- **链接跨盘隔离时是直接删掉链接本身**（不能还原，还原时会说明）；同卷隔离时链接本身可以移进隔离区并还原——同卷链接的还原没有单独写测试（同卷测试里没有链接）。
- **以管理员身份运行时的行为**没有验证（同前几个阶段，UAC 需要真人确认）。
- **UI Automation 设置 `<select>` 的值**是用 ExpandCollapse + SelectionItem 完成的；真鼠标 / 键盘操作没有验证（同阶段 C，模拟鼠标在这台机器上无效）。
- 隔离区批次号里的时间戳是 **UTC**（为了不依赖时区库），界面上显示的批次时间是本地时间。
- “彻底清空”在 GUI 里只截了二次确认，没有真的点“确定彻底删除”（点了“算了”再还原）；彻底清空的行为由 `quarantine_purge_removes_only_that_batch` 真实验证。

## 8. 磁盘空间

- 构建全部在 `C:\Users\king\AppData\Local\DiskWhy-build\target`（结束时 7.0 GB）。D 盘上只在测试 / 演示期间建过 `D:\desk\claudex\13\dw-xvol-test`（几十 MB 假文件，已删除）。
- 结束时：C: 可用 16.84 GB，D: 可用 35.52 GB。
