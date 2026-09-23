# 阶段 A 验证记录：扫描引擎 + 归属统计 + 可用的壳

日期：2026-09-23　机器：Windows 11 Pro 10.0.26200，8 核 16 线程，C 盘 NTFS 199 GB（簇 4096 B）
工具链：rustc/cargo 1.93.0，node 22.18.0，npm 10.9.3，Tauri 2（create-tauri-app 4.7.4 生成），windows-sys 0.61.2
以下所有输出都是本机实际运行的原文摘录。**所有运行均为普通用户（未提权）**。

---

## 1. 实现摘要

| 要求 | 实现位置 | 说明 |
|---|---|---|
| 多线程、每目录一个任务 | `crates/diskwhy-core/src/scan.rs` | rayon 专用线程池（逻辑核 ×2，上限 32，线程栈 8 MB），子目录 `into_par_iter()` 递归，work-stealing |
| 进度回调 ≤200ms、可取消 | 同上 `scan_with` | 独立进度线程，默认每 100ms 回调一次（`progress_interval_ms` 被钳在 10–200）；`AtomicBool` 取消，结果 `Err(ScanError::Cancelled)` |
| logical / allocated | `scan.rs::allocated_for`、`win.rs::compressed_size` | logical = `nFileSizeHigh<<32 \| nFileSizeLow`；allocated = `GetCompressedFileSizeW` 结果按簇（`GetDiskFreeSpaceW` 的 每簇扇区×每扇区字节）向上取整；取不到时退回 logical（同样按簇取整） |
| 目录访问失败不中断 | `scan.rs::scan_dir` | `FindFirstFileExW` 失败 → 节点 `inaccessible=true`、`error_code`，计入 `Attribution`，继续扫其余目录 |
| reparse point 不递归 | 同上 | 带 `FILE_ATTRIBUTE_REPARSE_POINT` 的目录只记自身（`reparse_skipped=true`）。**例外**：reparse tag 为云文件（`IO_REPARSE_TAG_CLOUD*`，`(tag & ~0xF000) == 0x9000001A`）的目录会进入枚举，否则 OneDrive 整个目录都会漏算；枚举目录不下载文件内容 |
| 云占位只看属性 | `scan.rs::file_node` | `OFFLINE \| RECALL_ON_OPEN \| RECALL_ON_DATA_ACCESS` 任一 → `cloud_placeholder=true`，**不调用 GetCompressedFileSizeW**（它内部会打开文件），本地占用按 0 计，单独统计个数和逻辑大小 |
| 长路径 | `win.rs::long_path` | 所有 API 路径加 `\\?\` 前缀，测试覆盖 >260 字符 |
| 系统保留按名归类 | `scan.rs::SYSTEM_RESERVED_NAMES` | 仅当扫描根是卷根时，根下的 `$Recycle.Bin`、`System Volume Information`、`pagefile.sys`、`hiberfil.sys`、`swapfile.sys`、`Recovery`、`$WinREAgent`（大小写不敏感）及其子树记为系统保留 |
| Attribution | `crates/diskwhy-core/src/attribution.rs` | 见第 5 节 |
| 树 + 文件聚合 | `scan.rs` | Dir 节点全部保留；每个目录只保留前 N（默认 50）大文件，其余合并成一个 `Rest` 节点（总量不变） |
| 管理员检测 / 提权重启 | `win.rs::is_elevated`、`relaunch_as_admin` | `OpenProcessToken` + `GetTokenInformation(TokenElevation)`；`ShellExecuteW("runas", 自身 exe)`，返回值 >32 成功，`ERROR_CANCELLED` 识别为用户点了“否” |
| 选盘 | `win.rs::list_fixed_drives` | `GetLogicalDrives` + `GetDriveTypeW == DRIVE_FIXED(3)` + `GetVolumeInformationW`（卷标/文件系统）+ `GetDiskFreeSpaceExW` |

API 签名均在动手前查过 learn.microsoft.com（FindFirstFileExW、WIN32_FIND_DATAW、GetCompressedFileSizeW、GetDiskFreeSpaceW/ExW、GetDriveTypeW、GetVolumeInformationW、File Attribute Constants、Reparse Point Tags、IsUserAnAdmin、TOKEN_ELEVATION、ShellExecuteW）和 docs.rs windows-sys 0.61.2；Tauri 部分查了 v2.tauri.app 的 create-project / calling-rust / calling-frontend（Channel）/ capabilities。
关于 `IsUserAnAdmin`：官方文档注明它“may be altered or unavailable”，建议直接用令牌查询，所以实现用的是 `TokenElevation`。

---

## 2. `cargo test -p diskwhy-core`

```
     Running unittests src\lib.rs (target\debug\deps\diskwhy_core-72130a56e7f29422.exe)
running 5 tests
test attribution::tests::gap_falls_back_to_system_reserved ... ok
test attribution::tests::non_root_scope_is_unattributed ... ok
test attribution::tests::gap_unattributed_when_everything_readable ... ok
test attribution::tests::gap_goes_to_inaccessible_when_other_dirs_denied ... ok
test attribution::tests::overcount_when_measured_exceeds_used ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running tests\mock_tests.rs (target\debug\deps\mock_tests-9ea6f355e2eb9e23.exe)
running 7 tests
test mock_root_denied_is_error ... ok
test mock_system_reserved_fallback_when_only_system_dirs_denied ... ok
test mock_volume_sizes_placeholders_reparse_and_system_reserved ... ok
test mock_top_n_aggregation ... ok
test view_listing_and_top_dirs ... ok
test mock_cancel_mid_scan ... ok
test mock_progress_interval_at_most_200ms ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.13s
     Running tests\real_fs_tests.rs (target\debug\deps\real_fs_tests-b5a7041295f3ed70.exe)
running 6 tests
test real_sizes_and_cluster_rounding ... ok
test real_offline_attribute_counted_as_placeholder_never_opened ... ok
test real_junction_not_followed ... ok
test real_long_path_beyond_max_path ... ok
test real_ntfs_compressed_file_uses_compressed_size ... ok
test real_access_denied_dir_is_marked_and_scan_continues ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
```

### 测试分两类，哪些是真实文件系统、哪些是 mock

**真实临时目录 + 真实 Win32 API**（`tests/real_fs_tests.rs`，在 `%TEMP%` 下建树，测完清理）：

| 测试 | 构造方法 | 断言 |
|---|---|---|
| `real_sizes_and_cluster_rounding` | 5000 B、1 B、100000 B、12288 B 文件 + 空目录，三层 | Exact/Fast 两种模式下：文件数 4、目录数 3、logical 合计；allocated = 按簇取整（1 字节文件 = 1 簇）；目录 allocated = 子项之和；非卷根时差额全部进“未归属”且各项之和 == 卷已用 |
| `real_ntfs_compressed_file_uses_compressed_size` | 1 MB 全零文件，`compact /c` 做 NTFS 压缩 | 带 COMPRESSED 属性，allocated < logical/4 且是簇的整数倍——证明确实用了 GetCompressedFileSizeW |
| `real_junction_not_followed` | `mklink /J link <另一个目录>`，目标里放 2 MB 文件 | link 节点 `reparse_skipped`、tag = `0xA0000003`、无子项、allocated 0；目标文件从未被查询 |
| `real_access_denied_dir_is_marked_and_scan_continues` | `icacls locked /deny *S-1-1-0:(RD)`（对 Everyone 拒绝列目录，测后恢复） | `inaccessible=true`、`error_code=5`、`inaccessible_total=1`，其余目录照常扫完（文件数 2） |
| `real_offline_attribute_counted_as_placeholder_never_opened` | `attrib +O` 设置 FILE_ATTRIBUTE_OFFLINE | 标记云占位、allocated 0、logical 保留、`cloud_placeholder_files=1`；用包装后端记录调用，证明**从未对它调用 GetCompressedFileSizeW**，而旁边普通文件被调用了 |
| `real_long_path_beyond_max_path` | 4 层 100 字符目录（>260） | 正常扫到，无无权限目录 |

**内存 mock 后端**（`tests/mock_tests.rs`，实现同一个 `FsBackend` trait）用于真实环境里普通用户造不出来或不稳定的情形：

- `RECALL_ON_OPEN` / `RECALL_ON_DATA_ACCESS` 只能由内核态（云同步驱动）设置，用户态无法在临时目录里造，所以用 mock；
- 卷根语义（`M:\` 下的 `$Recycle.Bin`、`System Volume Information`、`pagefile.sys`）和“系统保留目录无权限 → 差额兜底”需要一个卷根，临时目录不是卷根；
- 云目录（`IO_REPARSE_TAG_CLOUD | 0x3000`）要进入、符号链接目录（`0xA000000C`）不进入——mock 里符号链接后面挂了一个 1 TB 文件，如果误跟随会立刻暴露；
- 进度间隔：400 个目录、每次 list 延迟 20ms、4 线程（约 2 秒），断言相邻回调间隔 ≤250ms（要求 200ms，留 50ms 调度余量；实际配置为 100ms），最后一次回调的文件数与结果一致；
- 取消：进度回调里看到 ≥20 个文件就置取消位，断言返回 `Cancelled` 且 1.5 秒内返回；
- 前 N 聚合：120 个文件、N=50 → 51 个子项，Rest 节点 70 个文件，总 allocated 不变，保留的是最大的 50 个；
- 根目录无权限 → `RootUnavailable{code:5}`。

---

## 3. `cargo build --release`

```
   Compiling diskwhy v0.1.0 (D:\desk\claudex\13\diskwhy\src-tauri)
    Finished `release` profile [optimized] target(s) in 1m 42s

Name             Length
----             ------
diskwhy-cli.exe  424960
diskwhy.exe     8003584
```

GUI 发布版用 `npm run tauri build -- --no-bundle` 构建（`Built application at: D:\desk\claudex\13\diskwhy\target\release\diskwhy.exe`）。
注意：直接 `cargo build --release` 产出的 `diskwhy.exe` 没有打开 Tauri 的 `custom-protocol` 特性，会去连 dev server（localhost:1420），所以可运行的 GUI 必须用 tauri CLI 构建。`--no-bundle` 是为了不让 CLI 去下载 NSIS 打安装包。

---

## 4. CLI 对真实 C:\ 的扫描

命令：`target\release\diskwhy-cli.exe scan C:\ --top 30 --mode exact --json scan-final.json --quiet`
（外层 PowerShell 同时测了墙钟时间和峰值工作集）

```
DiskWhy 扫描结果：C:\
耗时 41.0 秒 | 文件 1211501 | 目录 219094 | 无权限目录 115 | 管理员 否 | 大小模式 Exact | 簇 4096 B
卷：总容量 199.03 GB | 已用 183.05 GB (196545679360 B) | 可用 15.99 GB

前 30 大目录（按实际占用 allocated，占比 = 占卷已用空间）：
   #           占用      占比  路径
   1    116.16 GB   63.5%  C:\Users
   2    116.16 GB   63.5%  C:\Users\king
   3    102.33 GB   55.9%  C:\Users\king\AppData
   4     65.46 GB   35.8%  C:\Users\king\AppData\Local
   5     36.31 GB   19.8%  C:\Windows
   6     30.44 GB   16.6%  C:\Users\king\AppData\Roaming
   7     15.95 GB    8.7%  C:\Program Files
   8     12.38 GB    6.8%  C:\Windows\System32
   9     12.00 GB    6.6%  C:\Users\king\AppData\Local\Packages
  10     11.99 GB    6.5%  C:\Windows\WinSxS
  11     10.60 GB    5.8%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc
  12     10.60 GB    5.8%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache
  13     10.28 GB    5.6%  C:\$Recycle.Bin [系统保留]
  14     10.28 GB    5.6%  C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001 [系统保留]
  15     10.28 GB    5.6%  C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV [系统保留]
  16     10.08 GB    5.5%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming
  17     10.05 GB    5.5%  C:\Users\king\AppData\Roaming\Claude
  18     10.05 GB    5.5%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude
  19      9.58 GB    5.2%  C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV\vm_bundles [系统保留]
  20      9.58 GB    5.2%  C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV\vm_bundles\claudevm.bundle [系统保留]
  21      9.19 GB    5.0%  C:\ProgramData
  22      8.18 GB    4.5%  C:\Users\king\AppData\Roaming\Claude\vm_bundles
  23      8.18 GB    4.5%  C:\Users\king\AppData\Roaming\Claude\vm_bundles\claudevm.bundle
  24      8.18 GB    4.5%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles
  25      8.18 GB    4.5%  C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle
  26      7.60 GB    4.2%  C:\Users\king\AppData\Local\Docker
  27      7.57 GB    4.1%  C:\Users\king\AppData\Local\Docker\wsl
  28      7.48 GB    4.1%  C:\Users\king\AppData\Local\Docker\wsl\disk
  29      6.81 GB    3.7%  C:\Windows\System32\DriverStore
  30      6.81 GB    3.7%  C:\Windows\System32\DriverStore\FileRepository

已用空间归属（卷已用 183.05 GB）：
  已扫描              182.45 GB   99.7%  
  无权限                    0 B    0.0%  差额推算；无权限目录 115 个（系统保留下 2 个），含 NTFS 元数据
  系统保留              10.53 GB    5.8%  按名字量到 10.53 GB，差额兜底 0 B
  云占位                    0 B    0.0%  0 个文件，逻辑大小 0 B，未打开、本地按 0 计
  未归属                    0 B    0.0%  
  重复计数           -   9.94 GB  硬链接等重复计数（阶段 C 去重）
  合计 196545679360 B == 卷已用 196545679360 B（差额原始值 gap = -10671628288 B）

JSON 已写入 ...\scratchpad\scan-final.json（目录树截到 3 层）

exit=0 wall=41.2s peakWS=231MB
```

- **耗时 41.0 秒**（普通权限，满足 PLAN 成功标准“< 90 秒”），121 万文件、21.9 万目录，峰值内存约 231 MB。JSON 11.6 MB（树截到 3 层 + 前 30 目录 + 前 200 条无权限目录）。
- 同一台机器上重复跑了 3 次 Exact：39.1 s（冷缓存）/ 40.2 s / 41.0 s。

### 4.1 与 `Get-PSDrive C` 对比

扫描结束后立即执行：

```
Get-PSDrive C   →  Used : 196555804672   Free : 17154465792
Get-Volume C    →  Size : 213710270464   SizeRemaining : 17154465792
```

| 来源 | 已用字节 |
|---|---|
| DiskWhy（GetDiskFreeSpaceExW：total − TotalNumberOfFreeBytes，扫描开始时取） | 196,545,679,360 |
| Get-PSDrive C（扫描结束后约 1 秒取） | 196,555,804,672 |
| 差 | 10,125,312 B ≈ 9.7 MB |

卷已用口径一致（总容量 213,710,270,464 B = 199.03 GiB 两边相同）；9.7 MB 是扫描期间系统和其他程序继续写盘造成的（两次取值相隔约 41 秒）。

### 4.2 差额去向（为什么“扫到的”比“已用”还多 9.94 GB）

扫描量出的三项之和 = 已扫描 182.45 GB + 系统保留 10.53 GB + 云占位 0 = 192.98 GB，比卷已用 183.05 GB 多，`gap = -10,671,628,288 B`。所以本次“无权限”和“未归属”都只能记 0，多出的部分记为 `overcount`（重复计数）。原因都查到了：

1. **Claude 商店应用的目录映射（约 10.05 GB 重复）**：`C:\Users\king\AppData\Roaming\Claude` 和 `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` 都是 10.05 GB。前者不是 reparse point（属性只有 Directory），但对其中的 `rootfs.vhdx` 执行 `fsutil hardlink list` 只返回一个名字：
   ```
   \Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle\rootfs.vhdx
   ```
   说明两个路径看到的是同一份文件——这是 MSIX 包的文件系统重定向（绑定过滤器），不是硬链接，也不带 reparse 属性，所以目录枚举看起来是两份。
2. **WinSxS 硬链接**：`C:\Windows\WinSxS`（11.99 GB）与 `System32` 等目录之间大量硬链接，阶段 A 按 PLAN 不去重（TODO 已记在 `Attribution.overcount` 注释中，阶段 C 用 `nNumberOfLinks > 1` / FileId 去重）。
3. **被抵消掉的“看不到”的部分**：115 个无权限目录（全部是错误码 5 拒绝访问），包括 `C:\System Volume Information`（卷影副本）、`C:\Program Files\WindowsApps`、`C:\Windows\System32\config`（注册表）、`C:\Windows\Prefetch`、`C:\ProgramData\Packages`、`C:\Users\CodexSandboxOffline`、`C:\$Recycle.Bin\S-1-5-18` 等，再加上 NTFS 元数据（$MFT 等）。它们真实占用为正，但被上面 1、2 两项的重复计数盖过了。

结论：四（五）项加总恒等于卷已用（不变式 `scanned + inaccessible + system_reserved + cloud_placeholder + unattributed − overcount == used` 在输出里逐字节验证为 `==`）；但在去重之前，本机“无权限目录到底占多少”无法从差额算出来，界面和 CLI 都明确提示了这一点，没有把它说成 0 就算完。

### 4.3 Exact 与 Fast 模式对比（为什么默认 Exact）

`--mode fast` 只对带 COMPRESSED / SPARSE / REPARSE 属性的文件调用 GetCompressedFileSizeW，其余按逻辑大小按簇取整。同机结果：

```
Fast : 耗时 4.4 秒 | 文件 1199514 | 已扫描 182.28 GB | 重复计数 - 10.10 GB
Exact: 耗时 40.2 秒 | 文件 1199748 | 已扫描 181.68 GB | 重复计数 -  9.49 GB
```

Fast 多算约 0.6 GB。逐目录比对发现差在 `C:\Program Files (x86)\Microsoft\Edge`、`EdgeCore`、`EdgeWebView`（各 188.4 MB）和 `Google\Chrome`（62 MB）：例如 `Locales\af.pak` 长度 1,063,016 B，GetCompressedFileSizeW 返回 352,256 B，但目录枚举给出的属性只有 `Archive`——这些是 WOF 压缩文件，WOF 过滤驱动在目录枚举里隐藏了它们的 reparse 属性，Fast 模式因此认不出来。所以默认用 Exact（41 秒，仍在 90 秒目标内），Fast 仅作为可选项保留。

---

## 5. 归属规则（Attribution）

量得到的：`scanned`（普通文件 allocated）、`system_reserved`（按名字归类后量到的）、`cloud_placeholder`（按 0 计，另记个数与逻辑大小）。
差额 `gap = used − 上述三项`，按以下顺序分配（`attribution.rs` 有单元测试逐条覆盖）：

1. `gap < 0` → `overcount = −gap`（重复计数），其余差额类为 0；
2. 扫描根不是卷根 → `unattributed = gap`（扫描范围外）；
3. 存在非系统保留的无权限目录 → `inaccessible = gap`（此时差额里还混着系统保留目录读不到的部分和 NTFS 元数据，普通权限下无法再拆，界面写明“差额推算，含 NTFS 元数据”）；
4. 只有系统保留目录无权限 → 用 GetDiskFreeSpaceEx 差额兜底：`system_reserved += gap`，同时记 `system_reserved_estimated`；
5. 没有任何无权限目录 → `unattributed = gap`。

---

## 6. GUI 截图（`target\release\diskwhy.exe`，普通权限）

截图方法：PowerShell 调 `PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT)` 抓窗口；按钮和列表行通过 UI Automation 的 `InvokePattern` 点击（这台机器当前桌面对 `CopyFromScreen` 返回全黑、模拟鼠标点击无效，所以改用这两种方式；界面操作本身是真实触发的，走的就是用户点击的同一条路径）。系统为深色主题，界面自动跟随。

| 文件 | 内容 |
|---|---|
| `screenshots/phaseA-0-start.png` | 启动页：选盘下拉（本地磁盘 (C:) 已用 182.8 GB / 199.0 GB）、开始扫描按钮、非管理员提示条 + “以管理员重新启动”按钮 |
| `screenshots/phaseA-1-scanning.png` | 扫描中：进度条、已扫文件/目录数、已量出字节、无权限目录数、用时、当前路径、取消按钮 |
| `screenshots/phaseA-2-result.png` | 扫描完成：顶部归属堆叠条（已扫描 / 系统保留 / 无权限 / 云端占位 / 未归属）+ 重复计数提示；C:\ 子项按实际占用降序，显示大小、占父目录百分比、类型，系统保留项带标签 |
| `screenshots/phaseA-3-drilldown.png` | 下钻一层到 `C:\Users`：面包屑、子项列表（junction `All Users` / `Default User` 显示“未跟随”，`CodexSandboxOffline` 显示“无权限”）、右侧详情为当前目录 |
| `screenshots/phaseA-4-detail.png` | 继续下钻到 `C:\Users\king\AppData\Local\Docker\wsl\disk` 并选中 `docker_data.vhdx`：详情面板显示路径、实际占用、逻辑大小、修改时间、属性，以及“说明（阶段 B 接入）”预留区 |
| `screenshots/phaseA-5-cancelled.png` | 点“重新扫描”后 3 秒点“取消扫描”，界面提示“扫描已取消”并回到初始页 |

GUI 内扫描一次 C:\ 用时 46.6–47.1 秒（与 CLI 相比多出的几秒是界面线程与进度 Channel 的开销，未做优化）。

---

## 7. 没做到 / 没验证的

- **“以管理员重新启动”没有端到端验证**：代码路径（`ShellExecuteW` runas + 成功后 `app.exit(0)`）已实现并编译通过，但点下去会弹出 UAC 安全桌面，需要真人确认，本次自动化会话里没有触发它。因此也**没有管理员身份下的扫描数据**。
- **硬链接 / 目录映射未去重**：按 PLAN 留到阶段 C，本机因此出现 9.94 GB 的重复计数（见 4.2）。
- 真实的 `RECALL_ON_OPEN` / `RECALL_ON_DATA_ACCESS` 占位文件只在 mock 里测过（本机没有 OneDrive 占位文件：真实扫描 `cloud_placeholder_files = 0`）；`OFFLINE` 用 `attrib +O` 在真实文件上测过。
- 只支持盘符路径（`X:\...`），UNC 路径和挂载到目录的卷未支持。
- 文件名里若有不成对的 UTF-16 代理项，会被有损转换（`from_utf16_lossy`），这类文件的路径在详情里可能对不上（极少见）。
- 没有用 `npm run tauri dev` 截图，截图全部来自发布版 exe；dev 模式下“以管理员重新启动”会让 dev server 随原进程退出，这个功能只在发布版里有意义。
- 没有 NSIS 安装包（`--no-bundle`，避免构建时下载外部工具）。

---

## 8. 给阶段 B 的接口说明

**core 入口**（`crates/diskwhy-core`）：

```rust
// 扫描真实盘符/目录，返回完整树
pub fn diskwhy_core::scan::scan<F: FnMut(&ScanProgress) + Send>(
    root: &str, opts: &ScanOptions, cancel: &AtomicBool, progress: F,
) -> Result<ScanResult, ScanError>;

// 同上，文件系统后端可替换（测试 / 未来别的数据源）
pub fn diskwhy_core::scan::scan_with<B: FsBackend, F>(fs: &B, root: &str, volume: VolumeSpace,
    elevated: bool, opts: &ScanOptions, cancel: &AtomicBool, progress: F) -> Result<ScanResult, ScanError>;

// 视图
pub fn diskwhy_core::view::find_dir<'a>(tree: &'a Node, path: &str) -> Option<Vec<&'a Node>>; // 根→目标的链
pub fn diskwhy_core::view::listing(result: &ScanResult, path: &str, limit: usize) -> Option<DirListing>;
pub fn diskwhy_core::view::top_dirs(tree: &Node, n: usize) -> Vec<&Node>;
```

`ScanResult.tree` 就是整棵树。**Node 结构**（`model.rs`）：

```rust
pub struct Node {
    pub name: String,
    pub path: String,        // Dir 必有；File/Rest 在内存里为空，用 父.path + "\\" + name 拼（view::node_view 会拼好）
    pub kind: NodeKind,      // File | Dir | Rest（Rest = 某目录“其余文件”合计）
    pub logical: u64,
    pub allocated: u64,      // 目录为子树合计
    pub mtime: i64,          // Unix 秒，0 = 未知
    pub attrs: u32,          // 原始 dwFileAttributes
    pub reparse_tag: u32,
    pub inaccessible: bool,
    pub error_code: u32,
    pub reparse_skipped: bool,
    pub cloud_placeholder: bool,
    pub system_reserved: bool,
    pub files: u64,          // 子树文件数
    pub dirs: u64,           // 子树目录数
    pub children: Vec<Node>, // 按 allocated 降序
}
```

阶段 B 建议：新增 `crates/diskwhy-core/src/explain.rs`，提供 `fn explain(node: &Node, ctx: &ExplainCtx) -> Explanation`（数据模型见 PLAN 第 2 节），然后在 `view::NodeView` 上加一个 `explanation: Option<Explanation>` 字段——前端 `NodeView` 类型与 `DetailPanel.vue` 的“说明”区域就是为它预留的。

**前端怎么拿数据**（`src/api.ts`，Tauri 命令在 `src-tauri/src/lib.rs`）：

| 前端函数 | Tauri 命令 | 返回 |
|---|---|---|
| `getEnv()` | `get_env` | `{ elevated, version }` |
| `listDrives()` | `list_drives` | `DriveInfo[]` |
| `startScan(root, onProgress)` | `start_scan`（参数 `root`、`onProgress: Channel<ScanProgress>`） | `ScanSummary`（归属、计数、耗时，不含树） |
| `cancelScan()` | `cancel_scan` | — |
| `listDir(path, limit)` | `list_dir` | `DirListing { node, children: NodeView[], total_children, breadcrumb }` |
| `relaunchAsAdmin()` | `relaunch_as_admin` | 成功后进程退出 |

扫描结果以 `Arc<ScanResult>` 存在后端 `AppState` 里，前端按目录懒取（`list_dir`），不会把 120 万节点一次性序列化给 WebView。能力文件 `src-tauri/capabilities/default.json` 只开 `core:default`；自定义命令按 Tauri 2 规则默认对所有窗口可用。
