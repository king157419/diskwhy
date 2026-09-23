# 阶段 C 验证记录：方块图 + 完整归属 + 打磨

日期：2026-09-23　机器：Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB（簇 4096 B），屏幕 2560×1600、缩放 125%
工具链：rustc/cargo 1.93.0，node 22，Tauri 2.11.6（tauri-plugin-opener 2.5.5、tauri-plugin-dialog 2.7.3），windows-sys 0.61.2，flate2 1.1，vitest 5.0.1
以下输出都是本机实际运行的原文摘录。**所有运行都是普通用户（未提权）。**
构建产物目录按要求改到了 C 盘：`CARGO_TARGET_DIR=C:\Users\king\AppData\Local\DiskWhy-build\target`（Tauri CLI 直接认这个环境变量，`Built application at: C:\Users\king\AppData\Local\DiskWhy-build\target\release\diskwhy.exe`，不需要改 tauri.conf.json）。

---

## 0. 先说最重要的发现：阶段 A 的 “−10 GB” 里有一半是测试环境造成的

我（和阶段 A、B 的执行者）的命令行进程都是从 **Claude 桌面版（商店 / MSIX 打包应用）** 里启动的，会继承它的文件系统虚拟化。
在这种进程眼里，`%APPDATA%\X` 和 `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\X` 是**同一个目录**（目录文件 ID 相同），所以扫描时算了两遍：

```
PS> fsutil file queryfileid $env:APPDATA\Claude
File ID is 0x0000000000000000004f00000001183f
PS> fsutil file queryfileid $env:LOCALAPPDATA\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude
File ID is 0x0000000000000000004f00000001183f
```

用 WMI（`Win32_Process.Create`，进程由 WmiPrvSE 创建，不继承打包应用的上下文）在**容器外**再跑同一个 CLI，结果：`C:\Users\king\AppData\Roaming\Claude` 根本不存在（`Test-Path` = False，`fsutil` 报 “Error 2”），也没有任何映射目录（“商店应用目录映射 0 处”）。
**普通用户从资源管理器 / 开始菜单启动 DiskWhy 时是容器外的情况**，所以下面两种环境的数字都给出来，结论以容器外为准。

顺带一提：本次按要求放在 `C:\Users\king\AppData\Local\DiskWhy-build` 的构建目录，因为是在 Claude 容器里创建的，物理上其实在
`C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build`（容器外看不到前一个路径；卸载 / 重置 Claude 桌面版会连它一起删掉）。

---

## 1. 做了什么

| 要求 | 实现位置 | 说明 |
|---|---|---|
| 硬链接去重 | `crates/diskwhy-core/src/win.rs::file_info`、`scan.rs`、`dedup.rs` | 见 2.1 |
| 商店应用目录映射去重 | `dedup.rs::find_mappings` | 见 2.2 |
| WindowsApps 能量的部分先量上 | `fs.rs::known_children`、`win.rs`、`explain/registry.rs::package_root_folders`、`scan.rs` | 见 2.3 |
| `Attribution.hardlink_dedup` 等字段 | `model.rs` | 新增 `dedup_enabled / hardlink_dedup / hardlink_dedup_files / mapped_dedup / mapped_dirs`（都带 `#[serde(default)]`，旧快照能读）；`Node` 新增 `hardlink_dup / mapped`；`ScanResult` 新增 `mapped_dirs: Vec<MappedDir>` |
| 方块图 | `src/treemap.ts`（纯算法）、`src/components/TreemapView.vue`（Canvas 2D） | 见第 5 节 |
| 方块图数据 | `view.rs::treemap`、Tauri 命令 `get_treemap` / `node_info` | 每块带档位 + 一句话 + 所属软件；最多 3 层，每层限量，其余合计成 `hidden_bytes` |
| 快照 | `crates/diskwhy-core/src/snapshot.rs`；CLI `--snapshot` / `--from`；GUI “保存快照 / 打开快照”，`diskwhy.exe <快照>` 启动即打开 | 见第 6 节 |
| 管理员模式规则 | `rules/windows-system.toml` 新增 10 条 keep | 见第 7 节 |
| 阶段 B 未知项 | `rules/misc.toml` 新增 7 条 + `windows-system.toml` 2 条（Users\Default、Public） | 见第 8 节 |
| 参考链接可点击 | `tauri-plugin-opener`（Rust + npm 都装上了，联网正常），`capabilities/default.json` 加 `opener:default` | 详情面板里链接改成可点，旁边保留“复制” |
| API Key 明文提示 | `SettingsDialog.vue` | 黄色提示框：明文存在 settings.json、谁能读到、建议用本机 Ollama 或低额度 Key |

规则库现在 **198 条**（179 + 19）。依旧没有任何删除按钮。

---

## 2. 去重

### 2.1 硬链接

**先查文档**：`WIN32_FIND_DATAW`（FindFirstFileEx 的结果）里没有链接数，也没有文件 ID（成员只有属性、三个时间、大小、dwReserved0/1、文件名）；
链接数只能打开句柄后用 `GetFileInformationByHandle` 的 `BY_HANDLE_FILE_INFORMATION.nNumberOfLinks` 取；文档要求“文件 ID + 卷序列号”一起比较，ReFS 上 64 位 ID 不保证唯一，应改用 `GetFileInformationByHandleEx(FileIdInfo)` 的 `FILE_ID_INFO`（64 位卷序列号 + 128 位 ID）。

**关键点：不多打开一次文件。** 阶段 A 默认的 Exact 模式本来就对每个文件调 `GetCompressedFileSizeW`（它内部要打开文件）。我把它换成自己打开一次句柄（只要 `FILE_READ_ATTRIBUTES`），在同一个句柄上查：

1. `GetFileInformationByHandleEx(FileCompressionInfo)` 的 `CompressedFileSize` —— 用来代替 GetCompressedFileSizeW；
2. `GetFileInformationByHandle` —— 拿 `nNumberOfLinks`；
3. 只有链接数 > 1 时再查 `FileIdInfo` 得到去重用的 key（失败退回 `dwVolumeSerialNumber + nFileIndexHigh/Low`）。

打不开句柄时退回原来的 GetCompressedFileSizeW（链接数按 1，不参与去重）。换掉取大小的方法之前，先逐文件核对了两种取法完全一致（含 WOF 压缩的 Edge、NTFS 压缩、稀疏、硬链接）：

```
C:\Program Files (x86)\Microsoft\Edge: 文件 930，一致 930，不一致 0，取不到 0，链接数>1 的 925；合计 731547259 vs 731547259
C:\Windows\System32: 文件 19369，一致 19369，不一致 0，取不到 0，链接数>1 的 17307；合计 13261140373 vs 13261140373
C:\Windows\WinSxS: 文件 97730，一致 97730，不一致 0，取不到 0，链接数>1 的 39181；合计 12624317292 vs 12624317292
```
（`cargo test -p diskwhy-core --test dedup_tests cmp_dir -- --ignored --nocapture`，`DISKWHY_CMP_DIR` 指定目录；测试里另有自动跑的 `real_file_info_matches_get_compressed_file_size`。）

**记哪一份**：扫描时对链接数 > 1 的文件记下 (key, 目录, 名字, 占用)；扫描结束后按 key 分组，每组只让一个名字记账，其余名字 allocated 记 0、标 `hardlink_dup`（被合并进“其余小文件”的就从那个合计里扣），然后自底向上重算目录大小、重排子项。挑记账的那个名字：
- **不在 `\Windows\WinSxS\` 下的优先**——这样 WinSxS 显示的是它“独占”的部分（本机从 11.99 GB 变成 3.65 GB），与 Microsoft 文档“WinSxS 看起来比实际大，因为很多文件是硬链接”的说法一致；
- 其余按完整路径（小写）字典序最小的优先。结果与线程调度无关（测试里用 1 / 2 / 8 线程各跑一遍断言一致）。

**Fast 模式**：只对压缩/稀疏/reparse 文件和硬链接密集区（卷根下 `Windows`、`Program Files`、`Program Files (x86)`）的文件打开句柄，其余不去重——这是任务里给的退路，文档和测试里都写明了。默认仍是 Exact。

### 2.2 商店应用目录映射

对树里每个 `...\AppData\Local\Packages\<包>\LocalCache\{Roaming,Local}\X`，如果 `...\AppData\{Roaming,Local}\X` 也在树里，就各打开一次目录（`CreateFileW` + `FILE_FLAG_BACKUP_SEMANTICS`）比较文件 ID；相同就把 AppData 那一侧记 0、`mapped = true`、子项不再展开，空间记在 Packages 那一侧（数据真正在的位置），`ScanResult.mapped_dirs` 记下对应关系。界面上这一行显示“映射”标签，详情面板顶部写“这是商店应用的目录映射：和「…LocalCache\Roaming\Claude」是同一份数据”，并有“跳到真正记账的位置”按钮（截图 phaseC-6）。规则库里 `claude-desktop-*` 的说明也写了这件事。
只在后处理里对候选目录打开几十次，不影响扫描主循环。

### 2.3 WindowsApps：能量的先量上

`C:\Program Files\WindowsApps` 普通用户不能列目录，但每个包自己的安装目录是可读的。注册表
`HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\<包全名>` 的 `PackageRootFolder` 给出了当前用户已装的包目录（本机 255 个包，其中 96 个在 WindowsApps 下）。
扫描到 WindowsApps 拒绝访问时，就按这份清单把这些子目录扫上；节点仍标“无权限”（清单不完整：其他账户的包、残留、暂存都看不到），差额推算照常进行。本机这样量出了 **7.65 GB**，“无权限”的推算值相应变小。

---

## 3. 测试

### `cargo test --workspace`（最后一次，全部代码定稿后）

```
     Running unittests src\lib.rs (...\target\debug\deps\diskwhy_lib-7a9dd7bc1e321a67.exe)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\main.rs (...\target\debug\deps\diskwhy-82690c4535ee2d89.exe)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\main.rs (...\target\debug\deps\diskwhy_cli-52a81bcb08faa372.exe)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\lib.rs (...\target\debug\deps\diskwhy_core-950b2ef936908f65.exe)
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
     Running tests\dedup_tests.rs (...\target\debug\deps\dedup_tests-d2a9403e140b3a42.exe)
test result: ok. 5 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.28s
     Running tests\explain_tests.rs (...\target\debug\deps\explain_tests-79de9015bc2c2d80.exe)
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.12s
     Running tests\mock_tests.rs (...\target\debug\deps\mock_tests-ad98221b9ecf57f3.exe)
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.13s
     Running tests\real_fs_tests.rs (...\target\debug\deps\real_fs_tests-3468f3804edca2af.exe)
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s
```

**55 个 Rust 测试全绿**（阶段 A/B 的 47 个 + 本阶段 8 个），1 个 `#[ignore]` 是上面手动跑的大目录对照。本阶段新增：

| 测试 | 怎么测 |
|---|---|
| `real::real_mklink_hardlinks_counted_once` | 临时目录里写 300000 B 文件，**`cmd /C mklink /H`** 再建两个名字 + 一个普通文件；断言 `file_info` 链接数 = 3、三个名字 key 相同、普通文件不取 key；扫描合计只算一次、`hardlink_dedup` = 2 份、`a\big.bin` 记账、另两个 `hardlink_dup`、文件数仍是 4、归属不变式成立；Fast 模式（非密集区）和 `--no-dedup` 都是 3 份 |
| `real::real_file_info_matches_get_compressed_file_size` | 普通 / `compact /c` 压缩 / `fsutil sparse setflag` + 64 MB 稀疏 / 硬链接四种文件，`file_info` 与 GetCompressedFileSizeW 逐个相等 |
| `real::real_snapshot_roundtrip_and_explain_after_load` | 真实扫描（含中文名、深目录、硬链接、60 个小文件触发聚合）→ 存 gzip → 读回，**`assert_eq!(back, r)` 整个 ScanResult 逐字段相等**（含所有目录 path、硬链接标记、归属）；读回后 `listing` 带解释，`node_modules` 仍被启发式认出；CLI `--json` 风格的扁平 JSON 也能读 |
| `mock_dedup_hardlink_prefers_non_winsxs_and_maps_store_app_dirs` | mock 卷：WinSxS 与 System32 同一文件 → System32 记账、WinSxS 只剩独占；AppData\Roaming\App 与 Packages\…\LocalCache\Roaming\App 目录 ID 相同 → 映射、子项清空；同名但 ID 不同的 Other 不去重；合计 == 已用、overcount 0；关掉去重时重复计数恰好 115 簇 |
| `mock_dedup_is_deterministic_and_handles_rest_aggregation` | 三个名字指向同一文件，其中一个被聚合进“其余小文件”；1/2/8 线程结果一致，Rest 合计被正确扣减 |
| `mock_windowsapps_known_package_dirs_are_measured_even_though_parent_denied` | WindowsApps 拒绝访问 + 清单里两个包（一个已不存在）：存在的量上、不存在的跳过、节点仍是无权限、差额只剩量不到的部分 |
| `admin_only_system_dirs_are_all_keep` | 22 个管理员才看得到的路径（SVI 及其卷影子目录、WindowsApps 包、WinSxS 组件、System32\config、sru、ServiceProfiles、CSC、Prefetch、AppRepository、Hyper-V、ProgramData\Packages、Defender / ATP、Crypto、Diagnosis、搜索索引）全部 keep、无删除、由规则说明 |
| `former_unknowns_now_have_rules` | 阶段 B 的 7 个未知项都命中新规则且档位正确；`*-updater` 规则靠 `when_child` 认，没有 installer.exe 的同名目录不命中；查不到官方说明的规则里写明“没有找到 / 不确定” |

### 前端 `npx vitest run`（`src/treemap.test.ts`）

```
 RUN  v5.0.1 D:/desk/claudex/13/diskwhy

 Test Files  1 passed (1)
      Tests  7 passed (7)
   Start at  12:21:18
   Duration  218ms (transform 42%, import 33%, tests 20%, worker 5%)
```

7 个用例：论文例子（6,6,4,3,2,2,1 铺 6×4：面积成比例、不重叠、前两块是 3×2）；200 个随机块最差长宽比中位数 < 3；0/负值/零面积；**小于 6px 的块并进“其他”且其余块最短边都 ≥ 6px**、同层不重叠、子块在父块里；最多 3 层、够大的目录画成容器、“其他”自己也太小就不画；命中测试返回最深的块；**根层 200 块 × 每块 30 个子项的嵌套布局 < 50 ms**。
`vue-tsc --noEmit` 通过（`npm run build` 的一部分）。

---

## 4. CLI 对真实 C:\（普通权限，Exact 模式）

### 4.1 差额收敛

| 环境 | 模式 | 耗时 | 已扫描 | 重复计数 | 无权限（推算） | gap（卷已用 − 量到的） |
|---|---|---:|---:|---:|---:|---:|
| 容器外（WMI 启动，= 普通用户） | `--no-dedup` | 19.5 s | 184.48 GB | **−8.61 GB** | 0 | −9,243,447,296 B |
| 容器外 | 去重（默认） | 19.9 s | 167.64 GB | **0** | **8.24 GB** | **+8,848,818,176 B** |
| Claude 容器内 | `--no-dedup` | 46.2 s / 45.5 s | 196.81 GB | −20.97 GB | 0 | −22,517,051,392 B |
| Claude 容器内 | 去重（默认） | 47.8 s / 46.1 s | 163.75 GB | 0 | 12.08 GB | +12,971,384,832 B |

容器外去重那次的原文（`scan-outside-dedup.txt`）：

```
耗时 19.9 秒 | 文件 1105048 | 目录 224610 | 无权限目录 120 | 管理员 否 | 大小模式 Exact | 簇 4096 B
卷：总容量 199.03 GB | 已用 186.41 GB (200156561408 B) | 可用 12.62 GB
  22      7.65 GB    4.1%  C:\Program Files\WindowsApps
已用空间归属（卷已用 186.41 GB）：
  已扫描              167.64 GB   89.9%
  无权限                8.24 GB    4.4%  差额推算；无权限目录 120 个（系统保留下 2 个），含 NTFS 元数据
  系统保留              10.53 GB    5.7%  按名字量到 10.53 GB，差额兜底 0 B
  云占位                    0 B    0.0%  0 个文件，逻辑大小 0 B，未打开、本地按 0 计
  未归属                    0 B    0.0%
  去重：硬链接 169642 个多余名字、16.85 GB；商店应用目录映射 0 处、0 B（已从上面各项里扣除）
  合计 200156561408 B == 卷已用 200156561408 B（差额原始值 gap = 8848818176 B = +8.24 GB）
```

容器内去重那次的原文（`final-dedup-1.txt`，节选）：

```
耗时 47.8 秒 | 文件 1079616 | 目录 221930 | 无权限目录 116 | 管理员 否 | 大小模式 Exact | 簇 4096 B
卷：总容量 199.03 GB | 已用 186.36 GB (200101384192 B) | 可用 12.67 GB
已用空间归属（卷已用 186.36 GB）：
  已扫描              163.75 GB   87.9%
  无权限               12.08 GB    6.5%  差额推算；无权限目录 116 个（系统保留下 2 个），含 NTFS 元数据
  系统保留              10.53 GB    5.7%  按名字量到 10.53 GB，差额兜底 0 B
  去重：硬链接 169642 个多余名字、16.85 GB；商店应用目录映射 25 处、16.21 GB（已从上面各项里扣除）
    C:\Users\king\AppData\Roaming\Claude = C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude（10.06 GB）
    …（另 24 处，含 AppData\Roaming\Microsoft 69.76 MB、AppData\Local\DiskWhy-build 1.74 GB）
  合计 200101384192 B == 卷已用 200101384192 B（差额原始值 gap = 12971384832 B = +12.08 GB）
```

**结论（如实）**：
- **重复计数被消除了**：容器外 −8.61 GB → 0；容器内 −20.97 GB → 0。“无权限”格第一次显示出真实的推算值（容器外 8.24 GB），而不是被重复计数盖成 0。
- **±1 GB 这个目标没有达到，也没法在普通权限下验证。** 去重后剩下的 +8.24 GB 全部落在“无权限”里：120 个拒绝访问的目录（`System Volume Information` 的还原点 / 卷影副本、`System32\config` 注册表、`ProgramData\Microsoft\Search\Data` 搜索索引、Defender 的定义和扫描数据、`Windows\LiveKernelReports`、WindowsApps 里别的账户的包等）加上 NTFS 元数据（$MFT 等；本机 108 万个文件，$MFT 通常就在 1 GB 以上）。这些普通用户读不到，`fsutil fsinfo ntfsinfo C:`、`Win32_ShadowStorage` 也都要管理员（实测 “Error 5: Access is denied” / “初始化失败”）。所以我不能说“差额 ±1 GB”，只能说“多算的已经去掉，剩下的差额有明确归属且是正数”。要真正核对需要管理员扫描（见第 7 节，未验证）。
- 容器内比容器外多出的约 3.8 GB“无权限”，是容器视图把一部分真实 AppData 内容换成了包内副本（例如容器内看到的 `AppData\Roaming\Microsoft` 只有 69.76 MB），扫描器看不到被遮住的那部分——这是运行环境造成的，不是扫描器的问题。
- WindowsApps 包目录补量（2.3）的效果：同样在容器外、同样去重，补量前“无权限”16.59 GB（WMI 第一次运行，11:16），补量后 8.24 GB。

### 4.2 扫描耗时（要求：增加不超过 30%）

| 环境 | 不去重 | 去重 | 增加 |
|---|---|---|---:|
| 容器内（交替各跑两次） | 46.2 s、45.5 s | 47.8 s、46.1 s | 平均 +2.4% |
| 容器内（加 WindowsApps 补量之前，交替各两次） | 44.6 s、44.0 s | 45.4 s、46.5 s | 平均 +3.4% |
| 容器外 | 19.5 s | 19.9 s | +2% |

（容器外整体快一倍多，是因为没有打包应用的文件系统过滤层。）

### 4.3 快照

```
快照已写入 …\scratchpad/phaseC-final.dwsnap（14.05 MB，用时 13.67 秒）      ← 第一版
已读取快照：…\phaseC-final.dwsnap（完整快照，保存于 Unix 1790186441，程序版本 0.1.0），读取用时 0.53 秒
快照已写入 …\scratchpad/resave.dwsnap（14.05 MB，用时 0.31 秒）             ← 修正后
  合计 200104550400 B == 卷已用 200104550400 B（差额原始值 gap = 12969062400 B = +12.08 GB）
```

完整 C 盘快照：解压后 122,589,527 B 的 JSON（子节点省略 path，读时拼回），gzip 后 14.05 MB。第一版保存要 13.7 秒——serde_json 往 GzEncoder 里写了大量极小的片段；在压缩器**前面**再加一层 1 MB 缓冲后降到 0.31 秒（同一个快照 `--from … --snapshot …` 重新保存测的）。

---

## 5. 方块图（GUI）

- **算法**：`src/treemap.ts`，squarified（Bruls 等 2000）+ 嵌套（最多 3 层，容器有 17px 标题栏）；最短边 < 6px 的块反复并入“其他”直到稳定，“其他”本身也太小就不画；后端每层限量（300 / 80 / 40 个、占根 0 / 0.04% / 0.1% 以上），没发来的合计为 `hidden_bytes` 也进“其他”。
- **Canvas 2D**：矩形、文字、斜纹都画在一个 `<canvas>` 上（按 devicePixelRatio 缩放），只有悬停提示是一个浮层 div。颜色从 CSS 变量读，和列表色标一致：safe 绿 / review 黄 / keep 红 / unknown 灰，深层稍微提亮；系统保留项（回收站等）叠白色斜纹。
- **根层归属色块**：扫描根上额外加“无权限（推算）/ 系统保留（推算）/ 未归属 / 云端占位”（字节 > 0 的才画），深灰斜纹；单击在详情面板里显示解释（为什么量不到、包含哪些目录、“以管理员身份重新启动可以看到更多”、系统还原点和搜索索引怎么处理），并列出前 12 个拒绝访问的目录作为线索。
- **交互**：悬停提示（名字、路径、大小、占当前视图、档位 + 这是什么 + 属于谁）；单击选中（右侧详情联动，直接子项用列表数据，深层块调 `node_info` 取完整解释）；双击进入（深层小块则进入它所在的最近一个能进的目录）；“↑ 上一级”、面包屑、Backspace、Alt+←、鼠标后退键上钻。
- **键盘 / 读屏**：canvas 可聚焦，←→ 在根层块之间移动并显示提示，Enter 进入，空格选中；另有视觉隐藏的按钮列表（“方块 X 大小”“进入 X”）供读屏软件使用。
- **布局**：左栏“方块图 + 列表”上下并排（默认，58% / 42%）/ 只看方块图 / 只看列表，右栏详情 380px（窄于 1360px 时 340px）；顶部概览可“收起”。按 125% 缩放下 1600×1000 物理像素 = **1280×800 逻辑像素**截图，全部内容放得下、无外层滚动条。窗口大小变化由 ResizeObserver 触发重绘。
- **性能**：页脚实时显示“N 个根层块 · 布局+绘制 x ms”。截图里：C:\ 根层 9 块 1.1 ms；`AppData\Local` 48 块（加上嵌套共几百块）1.9 ms（只看方块图；上一次构建在并排视图下 45 块 14.7 ms）；`AppData\Roaming` 33 块 6.7 ms。200 块的布局在 vitest 里测过 < 50 ms。

---

## 6. 快照（GUI）

- “保存快照”：Rust 侧弹系统“另存为”对话框（tauri-plugin-dialog 的 `blocking_save_file`，默认文件名 `DiskWhy-C-<时间>.dwsnap`），写 gzip JSON。
- “打开快照”：弹“打开”对话框（过滤 `*.dwsnap;*.json`），读回后替换当前结果，**各档合计和每一项说明都用当前规则库现算**；顶部绿色条写明快照路径、保存时间、读取用时；顶栏显示“快照 · …”。
- `diskwhy.exe <某个快照.dwsnap>` 启动时直接打开（为双击关联文件准备，也方便自动化）。

**验证情况**：对话框本身我点不了——这台机器上 UI Automation 只能看到对话框里的控件（全是 Pane，没有 Value/Invoke 模式），`SendKeys` 返回 “Access is denied”，模拟鼠标对窗口无效（阶段 A 已记录）。所以：
- 点“保存快照”后用 UI Automation 确认弹出了标题为“保存扫描快照”的 `#32770` 对话框，文件名框里是 `DiskWhy-C-1790187564.dwsnap`，然后结束进程；**没能在 GUI 里完成一次保存**；
- “快照加载”截图（phaseC-7）用的是 `diskwhy.exe phaseC-final.dwsnap` 启动参数：走的是和“打开快照”按钮完全相同的 `open_snapshot` 命令（只是跳过对话框），快照由同一个 core 的 CLI `--snapshot` 生成。读取用时 0.5 秒。

---

## 7. 管理员模式

规则：已有的 `system-volume-information`、`windows-apps`、`winsxs`、`prefetch`、`search-index`、`windows-defender` 沿用（都是 keep、tree）；`%SystemRoot%` 整棵树由 `windows-dir`（keep、tree）兜底。新增 10 条 keep：`registry-hives`（System32\config）、`service-profiles`、`srum-database`（System32\sru）、`offline-files-csc`（Windows\CSC）、`programdata-windows`（ProgramData\Microsoft\Windows）、`appx-repository`、`programdata-packages`（写明“官方没写具体内容，我不确定”）、`defender-endpoint`、`machine-crypto-keys`、`windows-diagnosis`。`admin_only_system_dirs_are_all_keep` 测试覆盖。

**以管理员身份实际扫描：未验证。** 试了一次：

```
PS> Start-Process cmd.exe -ArgumentList '/c "…\diskwhy-cli.exe" scan C:\ … > D:\desk\claudex\13\dw-wmi\admin-scan.txt' -Verb RunAs
Start-Process -Verb RunAs 45 秒内没有返回（UAC 提示等待确认）
Test-Path D:\desk\claudex\13\dw-wmi\admin-scan.txt  →  False
```

UAC 确认框需要真人点“是”，我不能点，所以没有任何管理员身份下的扫描数据；GUI 的“以管理员重新启动”同样没有端到端验证（同阶段 A）。**注意**：那个 UAC 提示可能还停在你的屏幕上（`consent.exe` 进程还在）。如果你点了“是”，它会以管理员身份跑一次只读扫描，把结果写到 `D:\desk\claudex\13\dw-wmi\admin-scan.txt`——正好可以拿来核对第 4 节的“无权限 8.24 GB”。

---

## 8. 阶段 B 未知项的规则

| 路径 | 规则 / 档位 | 依据 |
|---|---|---|
| `%USERPROFILE%\.vscode` | `vscode-home-dir` review（self） | VS Code 官方文档：扩展在 `%USERPROFILE%\.vscode\extensions`，可用 `--extensions-dir` / `VSCODE_EXTENSIONS` 改位置；`cli` 子文件夹写明“官方文档没找到说明” |
| `%USERPROFILE%\.antigravity`（及 `.antigravity-ide`） | `antigravity-home-dir` review（self） | Antigravity 官方文档没写；引用的是 Google 开发者论坛的用户帖子，规则里注明“不是官方说明” |
| `%LOCALAPPDATA%\OpenAI\Codex` | `codex-desktop-local` review | openai/codex 仓库 issue #42412（桌面版把商店包里的运行时搬到这里）；“OpenAI 官方文档里没有这个文件夹的说明”，删了会怎样写“我不确定” |
| `~\.cache\codex-runtimes` | `codex-runtimes-cache` review | 没找到官方说明，只列线索（三套运行时、runtime.json、`.previous-<GUID>` 旧版本） |
| `%APPDATA%\ACLOS`、`%USERPROFILE%\AppData\ACLOS` | `aclos-game-recorder` review，所属写“不确定（可能与《无畏契约》的“无畏时刻”录像有关）” | 只有网上视频这么说，没有官方说明、注册表也没登记，规则里如实写 |
| `%LOCALAPPDATA%\*-updater`（kimi-desktop-updater 等） | `electron-updater-cache` **safe**，`when_child = [installer.exe, pending, update-info.json]` | 查了 electron-builder 源码：`appInfo.ts` 里 `updaterCacheDirName = 软件名小写 + "-updater"`，`AppAdapter.ts` 里 Windows 缓存根是 `LOCALAPPDATA`，`AppUpdater.ts` 把下载的更新放在这里；删了下次更新重新下载 |
| `%LOCALAPPDATA%\Pandoc` | `pandoc-local` review | pandoc.org 安装页只说 MSI 会安装并加 PATH，**没写默认位置**；本机注册表没登记；规则按文件内容认定并写明这些 |

另外补了 `C:\Users\Default`（新账户模板，keep）和 `%PUBLIC%`（公用文件夹，keep）——截图里 Users 下这两个原来显示“未知”。
新规则里的参考链接逐个 curl 过：electron-builder 两个源码页、Google 论坛帖、learn.microsoft.com registry-hives、VS Code extension-marketplace、pandoc installing、openai/codex issue #42412 全部 200。
本次扫描“规则还没覆盖”从 27.08 GB 降到 13.95 GB（顶部汇总卡）。

---

## 9. GUI 截图（`docs/screenshots/`，release 版，普通权限，窗口 1280×800 逻辑像素）

截图方法同前两阶段：UI Automation 点按钮 / 行，`PrintWindow(PW_RENDERFULLCONTENT)` 抓窗口。**画布上的操作**（选中块、悬停提示、进入）用的是方块图的隐藏按钮列表（读屏用的那组）——它们调用的就是键盘焦点那条路径：选中块 + 在块中心显示和鼠标悬停同一个提示浮层。模拟鼠标在这台机器上不生效（试过 SetCursorPos 移到方块上，提示没出现），所以没有“真鼠标悬停”的截图。

| 文件 | 内容 |
|---|---|
| `phaseC-1-root-treemap.png` | GUI 内扫描 C:\ 完成（47.4 秒）：顶部归属条（无权限 12.09 GB 为斜纹）+ “已去重 33.86 GB：硬链接 169,670 个多余名字 17.09 GB · 商店应用目录映射 25 处 16.77 GB”；方块图根层 9 块（Users 容器里嵌套 king → AppData、Windows → System32/DriverStore/WinSxS 等），右上深灰斜纹“无权限（推算）12.09 GB”，回收站叠斜纹；页脚“9 个根层块 · 布局+绘制 1.1 ms” |
| `phaseC-2-root-inaccessible-block.png` | 选中“无权限（推算）”块：蓝色描边 + 提示浮层；详情面板显示合成的解释（量不到的原因、包含哪些目录、以管理员身份重启可以看到更多） |
| `phaseC-3-hover-tooltip.png` | 聚焦 Windows 块：提示浮层（保留 · Windows · C:\Windows · 25.75 GB · 占当前视图 13.8% · Windows 系统本身 · 属于 Windows · 单击看详情 · 双击进入），列表里同一行同时高亮，右侧详情联动 |
| `phaseC-4-drilldown-users.png` | “进入 Users”后：面包屑 C:\ › Users、“↑ 上一级”可用；king → AppData → Local / Roaming / ACLOS 嵌套，`.claude`、`.vscode`、`.antigravity`、`.cache` 等按档位着色 |
| `phaseC-5-many-blocks-local-maponly.png` | 继续下钻到 `AppData\Local`，切到“只看方块图”：48 个根层块、三层嵌套，布局+绘制 1.9 ms；Temp、npm-cache、uv\cache 等为绿色，Docker / wsl vhdx 为红色 |
| `phaseC-6-mapped-dir.png` | `AppData\Roaming` 里的 `Claude` 行带“映射”标签、0 B；详情顶部：“这是商店应用的目录映射：和「C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude」是同一份数据……”+“跳到真正记账的位置”按钮 |
| `phaseC-7-snapshot-loaded.png` | 用 `diskwhy.exe phaseC-final.dwsnap` 打开 CLI 生成的快照：绿色条“正在查看快照：…phaseC-final.dwsnap（保存于 2026-09-23 12:00，读取用时 0.5 秒）”，顶栏“快照 · 用时 48.5 秒 · 1,079,688 个文件”，方块图、列表、各档合计（命中 93/198 条规则）都照常算出 |

截图是在 Users\Default / Public 两条规则加进去**之后**重新构建、重新截的。

---

## 10. 没做到 / 没验证的

- **“扫描合计与卷已用之差 ±1 GB”没有达到**：多算的部分已消除（→ 0），但剩下 +8.24 GB（容器外）是 120 个无权限目录 + NTFS 元数据的推算值，普通权限下无法拆开核对（见 4.1）。
- **管理员身份扫描未验证**（UAC 需要真人确认，见第 7 节）。
- **GUI 里没完成一次“保存快照”**：对话框弹出已确认，但填不了文件名（见第 6 节）；保存逻辑由 CLI 和 `real_snapshot_roundtrip` 测试覆盖。“打开快照”按钮的对话框同理，截图走的是启动参数。
- **真鼠标悬停 / 双击没有自动化验证**，截图里的提示和进入是键盘焦点路径触发的（代码里鼠标和键盘共用同一个提示与进入逻辑）。
- 目录映射只识别 `Packages\<包>\LocalCache\{Roaming,Local}\X` ↔ `AppData\{Roaming,Local}\X` 这一种（本次唯一遇到的）；其他形式的重复（例如别的过滤驱动造成的）不会被识别，会体现在“重复计数”里。
- Fast 模式只在 `Windows`、`Program Files*` 下去重硬链接；别处的硬链接（比如本测试临时目录里用 mklink /H 造的）在 Fast 模式下仍会重复计数。
- WindowsApps 只补量了**当前用户**注册的包目录；其他账户的包、暂存和已删除的包仍算在“无权限”里。
- 旧快照（阶段 A/B 格式或 `--json` 输出）读得进来，但没有去重信息，界面会提示“这份结果没有做去重”。
- API Key 仍然明文保存（按要求只加了界面提示，没加密）。
- 快照读写没有进度条（完整 C 盘保存 0.3 秒、读取 0.5 秒，GUI 里读取还要现算各档合计）。

## 11. 磁盘空间

- 构建全部在 `C:\Users\king\AppData\Local\DiskWhy-build\target`（物理位置见第 0 节）；过程中两次删掉了本项目自己的 `target\debug`（可再生的编译缓存，约 3.5–4.1 GB）以保持 C 盘余量。D 盘上没有新生成 target（原有的 `D:\desk\claudex\13\diskwhy\target` 1.8 GB 是阶段 A/B 留下的，没动）。
- 结束时 `df -h`：C: 可用约 13 GB，D: 可用约 3.5 GB（node_modules 新增 vitest、两个 Tauri 插件，约 30 MB）。
