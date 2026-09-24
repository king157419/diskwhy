阶段 B 是 DiskWhy 的“解释引擎”：给扫描出来的每个文件夹说清楚它是什么、属于谁、为什么在、删了会怎样、该怎么腾，只解释不删除。

# 阶段 B 验证记录：解释引擎

> 第 1–6 节是 2026-09-23 上午第一次做阶段 B 时的记录（当时 179 条规则、47 个测试）。当晚 S22 轮在分支 `phase-b` 上补做了 B-1、B-2、B-3，见文末第 7（决定）、8、9、10 节；第 3 节的覆盖率已被第 10 节和新的 coverage.md 取代。
> **注意**：B-2 改了几处第 1 节写过的行为——启发式现在可以对签名严格的 Chromium 缓存给 safe（1.4 节说“永不 safe”已过时）；LLM 请求体从 9 个字段收紧到 4 个、回复只收 what/why/lose、可信度固定 0.3（1.5 节已过时）。以第 9 节为准。

日期：2026-09-23　机器：Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
工具链：rustc/cargo 1.93.0，node 22，Tauri 2，windows-sys 0.61.2，toml 0.9，ureq 3.4.2（native-tls）
以下输出均为本机实际运行的原文摘录。覆盖率细节见 [coverage.md](coverage.md)。

---

## 1. 做了什么

| 要求 | 实现位置 | 说明 |
|---|---|---|
| 规则库 `rules/*.toml` | `rules/` 6 个文件，**179 条** | 编译期 `include_str!` 嵌入；运行时再加载 `%LOCALAPPDATA%\DiskWhy\rules\*.toml`（同样校验） |
| 规则加载 / 校验 / 匹配 | `crates/diskwhy-core/src/explain/{rules,pattern,env}.rs` | `%ENV%` 白名单展开、`*` `?` 段内通配、`**` 跨段通配、大小写不敏感、按“具体程度”选最优规则、上层 `scope=tree` 规则继承 |
| 启发式 | `explain/heuristics.rs` | 12 个内容签名 + 注册表归属 + 年龄提示 + 未知线索；结果永不为 safe |
| 注册表快照 | `explain/registry.rs` | `RegOpenKeyExW` / `RegEnumKeyExW` / `RegGetValueW` 读三处 Uninstall 键，启动时读一次（本机 260 个软件，12–14 ms） |
| LLM 兜底 | `explain/llm.rs` | 默认关闭；OpenAI 兼容 `/chat/completions`；只发名字/脱敏路径/子项名/扩展名分布/大小/日期 |
| 入口 | `explain::explain(&Target, &ExplainCtx) -> Explanation` | `ExplainCtx` = 环境变量 + 规则库 + 注册表快照 + LLM 设置 |
| 接入 view | `view::listing(result, path, limit, Some(ctx))` | `NodeView.explanation` 对当前目录和每个子项都算；另有 `view::find_node` / `view::explain_path` |
| 全盘各档合计 | `explain::tier_summary` | 按规则库把每个字节归到 safe / review / keep / 未覆盖（带前缀剪枝，本机全盘 145–153 ms） |
| CLI | `diskwhy-cli explain C:\ --top 50 [--json] [--md]` | `crates/diskwhy-cli/src/explain_cmd.rs` |
| Tauri 命令 | `src-tauri/src/lib.rs` | 新增 `get_settings` / `save_settings` / `test_llm` / `ask_ai`；`start_scan` 返回 `tiers`；`list_dir` 带解释；上下文懒加载一次后缓存 |
| GUI | `src/components/{DirList,DetailPanel,TierSummaryCard,SettingsDialog}.vue` | 列表色标 + 一句话、详情六项 + 来源 + 参考、未知线索 + 问 AI、顶部汇总卡、设置页。**仍然没有删除按钮** |

### 1.1 规则字段（在任务给的 schema 上的扩展，均为可选）

- `how` 可以是字符串，也可以是数组（AI 模型要同时写 `migrate` 和 `delete`）。
- `category` 在任务给的 9 种之外加了 `app`（软件本体 / 安装目录）。
- `scope = "tree"`（默认，子孙沿用本规则）或 `"self"`（只说明这一层，用于 `C:\Users`、`AppData\Local` 这类“容器”）。
- `kind = "any" | "dir" | "file"`；`when_child = [...]`：目录里必须有其中之一才算命中——Chromium 通配规则靠它避免误伤同名普通文件夹（`Cache` 里要有 `Cache_Data` / `data_0`，`Code Cache` 里要有 `js` / `wasm`……）。
- `owner_from_path = true`：通配规则（如 `%APPDATA%\**\Cache`）用注册表 / 文件夹名推断具体是哪个软件，界面显示“LarkShell（Chromium / Electron 程序）”。

### 1.2 启动校验（失败的规则不加载；内置规则有任何一条失败，测试就失败）

字段齐全（`deny_unknown_fields` + 无默认值；测试还直接按 TOML 表逐条检查 12 个必填字段）、枚举合法、id 唯一且只含 `[a-z0-9-]`、每个模式必须以白名单 `%变量%` 开头且不含 `..`、**safe 档的模式不许落在 `%USERPROFILE%` / `%PUBLIC%` 下的 Desktop / Documents / Pictures / Videos / Music / Downloads / OneDrive*（也不许在这一层用通配）**、**keep 档 `how` 不许含 delete**、`builtin_tool` / `migrate` 必须写 `how_detail`、`verified_on` 是 YYYY-MM-DD、`refs` 是网址。
另外运行时兜底：safe 规则如果展开后落在受保护目录（含 `%OneDrive%` 实际路径、文件夹重定向后的位置），自动降为 review 并附说明。

### 1.3 规则库构成

| 文件 | 条数 | safe | review | keep |
|---|---:|---:|---:|---:|
| windows-system.toml | 44 | 10 | 6 | 28 |
| browsers.toml | 19 | 9 | 1 | 9 |
| dev-caches.toml | 41 | 16 | 12 | 13 |
| ai-models.toml | 13 | 0 | 12 | 1 |
| cn-apps.toml | 36 | 7 | 12 | 17 |
| misc.toml | 26 | 5 | 12 | 9 |
| **合计** | **179** | 47 | 55 | 77 |

PLAN 第 3 节阶段 B 列的条目全部覆盖，并由测试 `required_items_from_plan_are_covered` 逐条断言档位和办法（WinSxS keep+DISM、Windows.old review+磁盘清理、Installer keep、Prefetch keep、hiberfil review+`powercfg /h off`、pagefile keep、两个 Temp safe、传递优化 safe+工具、Package Cache review、缩略图 safe、docker_data.vhdx 与 WSL ext4.vhdx keep+migrate（含 `wsl --export/--import`、Docker Disk image location）、微信/QQ 聊天文件 keep+migrate、微信渲染缓存/日志 safe、飞书 update safe 其余 review、网易云 Cache safe / 下载 keep、7 类 AI 模型 review+[migrate, delete]、conda pkgs safe+`conda clean --all`、conda envs keep+migrate、浏览器缓存 safe / Profile keep、Chromium/Electron 通配）。

**文案**全部自己用中文写；`what` 一句话，`lose` 都写了“删了会怎样、丢不丢数据”，拿不准的写了“我不确定”并给线索（飞书、钉钉、QQ 音乐、酷狗、罗技 G HUB、NVIDIA NGX、Claude vm_bundles 等）。没有看、也没有导入 MangoDisk 和 Winapp2。

**参考链接**：规则里一共 57 个不重复的 `refs` 网址，逐个用 curl 请求过：55 个 200，2 个 404（Ollama 文档搬家了）——已换成 `https://docs.ollama.com/faq` 和 `/troubleshooting`，并确认 FAQ 里写的就是 `OLLAMA_MODELS` 和默认的 `C:\Users\%username%\.ollama\models`。`wsl --manage <发行版> --move <位置>` 在 Microsoft Learn 的 basic-commands 页面上**没有**，是用本机 WSL 2.6.3 的 `wsl --help` 确认存在的，所以规则里写的是“新版 WSL 可直接……；旧版用 export/import”。

**关于 `verified_on`**：179 条都写的 2026-09-23，意思是“这天写的、这天查过文档”。其中本次扫描里真正命中过的有 93 条（`命中规则 93/179`），剩下 86 条的路径本机不存在（比如本机没装 QQ 音乐、酷狗、钉钉、迅雷、B 站客户端、搜狗、企业微信、JetBrains、Gradle、Maven；本机微信装在 `D:\Weixin` 且改过存储位置），这些路径来自我对这些软件默认位置的了解，**没有在本机上核实**，下一步需要在装了这些软件的机器上跑一遍。

### 1.4 启发式（规则没命中时，按顺序）

| 名字 | 判定 | 档位 |
|---|---|---|
| chromium-cache | 目录里有 `Cache_Data\index`，或 `index` + `data_0/1` | review |
| python-venv / python-env | 有 `pyvenv.cfg`（或名叫 `.venv`/`venv` 且有 site-packages）→ 虚拟环境；有 `site-packages` / `Lib\site-packages` / `lib\python*\site-packages` / `conda-meta` → Python 环境 | review |
| node-modules | 名叫 `node_modules` **且父目录有 package.json** | review |
| rust-target | 名叫 `target` 且旁边有 `Cargo.toml` | review |
| dotnet-build | `bin` / `obj` 同时存在且旁边有 `*.csproj/.fsproj/.vbproj` | review |
| model-weights | `.safetensors/.gguf/.onnx/.pt/.pth/.bin/.ckpt` 占字节 ≥ 60% 且最大的单个 > 100 MB | review，[migrate, delete] |
| virtual-disk | `.vhdx/.vhd/.vmdk/.qcow2/.avhdx/.vdi` 占字节 ≥ 50% | **keep**，migrate |
| logs-dumps | `.log/.etl/.dmp/.mdmp/.hdmp` 占字节 > 70% | review |
| downloaded-installer | 在 `%USERPROFILE%\Downloads` 下，`.iso/.msi/.exe/.msix…` 占 > 50% | review |
| temp-files | `.tmp` 按字节或个数占 > 50% | review |
| media-files（附加） | 视频/图片/音频占字节 > 70% 且 > 10 MB | review，manual |
| registry-install-dir | 路径落在注册表 `InstallLocation`（为空时用 `DisplayIcon` 的 exe 目录）之下 | review |
| registry-program-dir / registry-appdata | `Program Files`、`AppData\{Local,Roaming,LocalLow}`、`ProgramData`、`Packages` 下前三层文件夹名 vs DisplayName（先精确，再**唯一的**前缀匹配），再 vs Publisher；同一发布者装了多个软件时只说到厂商，不猜具体是哪个 | review |

- 名字规范化：小写、去括号内容、去版本号、去 Inc/Ltd/LLC/Corp/Corporation/GmbH/x64 等后缀、只留字母数字汉字；前缀匹配要求短的一方 ≥ 4 个字符。没用模糊匹配库。
- 扫描树每个目录只保留前 50 大文件，所以 `package.json` / `Cargo.toml` / `*.csproj` 在树里找不到、而父目录又有“其余小文件”聚合时，**退回去问文件系统**（只查存在与否，不读内容）。有专门测试。
- 子树统计最多看 30000 个节点（大目录采样），保证列表秒开。
- 年龄：子树里最新修改时间超过 180 天，附“已经 N 个月没动过了（最近一次修改在 …）”。
- 兜底：`finish()` 里强制 safe → review；未知 `source=Unknown`、`what="我也不确定这是什么"`，线索 = 最大的 5 个子项、主要扩展名分布、文件/文件夹数、修改时间、是否在 AppData 下、注册表里有没有对得上的软件。

### 1.5 LLM 兜底

- 设置：`%LOCALAPPDATA%\DiskWhy\settings.json`，`{enabled, base_url, model, api_key}`，默认关闭，坏文件退回默认。
- 请求体 `LlmInput` 只有 9 个字段：名字、父路径（任何 `X:\Users\<名字>` → `~`）、类型、总大小、文件数、文件夹数、前 10 大子项名和大小、扩展名分布、修改日期。结构体里根本没有“内容”字段。
- 系统提示要求只输出 `{what, owner, why, lose, confidence}` JSON，并写明“这是推断不是确认，拿不准就说不确定”。解析时容忍 ```json 包裹；结果一律 `tier=review`、`source=Llm`、可信度上限 0.9，界面标“AI 推测 · 需要你来判断”。
- 只在详情面板点“问 AI”时调用；LLM 没启用时按钮变成“问 AI（先去设置里启用）”并打开设置页。所有网络错误都转成中文字符串显示在面板里，不弹窗、不影响其他功能。
- ureq 3 默认 TLS 是 rustls；我们只开了 `native-tls`（Windows 上走 SChannel），所以代码里显式指定 `TlsProvider::NativeTls`（查过 ureq 3.4.2 源码里 `TlsProvider` 的默认值和 feature 定义）。

---

## 2. `cargo test`（core + cli）

```
     Running unittests src\main.rs (target\debug\deps\diskwhy_cli-c9af303e0273314d.exe)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\lib.rs (target\debug\deps\diskwhy_core-6b210520c6e2753f.exe)
test explain::pattern::tests::double_star ... ok
test explain::pattern::tests::star_within_segment ... ok
test explain::pattern::tests::pruning ... ok
test explain::registry::tests::normalize_names ... ok
test explain::pattern::tests::specificity_prefers_more_literal ... ok
test explain::env::tests::expand_basic_and_case_insensitive_names ... ok
test explain::registry::tests::install_location_from_icon_and_shallow_filter ... ok
test explain::pattern::tests::literal_case_insensitive ... ok
test explain::registry::tests::real_registry_has_some_apps ... ok
（另 5 个阶段 A 的 attribution 测试 ok）
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
     Running tests\explain_tests.rs (target\debug\deps\explain_tests-b8fd7a90f9697ea7.exe)
test llm_parse_and_always_review ... ok
test redact_path_rules ... ok
test name_normalization_is_simple_and_symmetric ... ok
test llm_settings_roundtrip_and_default_off ... ok
test every_rule_has_all_fields_in_raw_toml ... ok
test safe_rules_never_under_personal_folders_and_keep_never_delete ... ok
test embedded_rules_all_valid_and_enough ... ok
test required_items_from_plan_are_covered ... ok
test tier_summary_splits_bytes_by_rule ... ok
test missing_env_var_skips_pattern_but_keeps_rule ... ok
test env_expansion_case_and_wildcards ... ok
test user_rules_dir_is_loaded_and_validated ... ok
test llm_request_body_has_no_file_content_and_redacted_path ... ok
test validation_rejects_bad_rules ... ok
test heuristic_registry_owner_and_age ... ok
test listing_attaches_explanation_to_every_child ... ok
test llm_end_to_end_against_local_fake_server ... ok
test heuristic_signatures_on_real_dirs ... ok
test heuristic_sibling_marker_found_even_when_parent_files_aggregated ... ok
test llm_network_failure_is_graceful ... ok
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.06s
     Running tests\mock_tests.rs  → test result: ok. 7 passed
     Running tests\real_fs_tests.rs → test result: ok. 6 passed
```

合计 **47 个测试全绿**（阶段 A 的 18 个 + 阶段 B 新增 29 个）。阶段 B 测试要点：

| 测试 | 怎么测 |
|---|---|
| `embedded_rules_all_valid_and_enough` | 内置 6 个文件校验零错误、≥ 80 条、每组都有、每个模式都能展开 |
| `every_rule_has_all_fields_in_raw_toml` | 不经 serde，直接按 TOML 表检查 12 个必填字段 |
| `validation_rejects_bad_rules` | 13 种违规写法（safe 落在文档/桌面/OneDrive/下载、safe 用 `%USERPROFILE%\**`、keep+delete、未知变量、非变量开头、`..`、migrate 无 how_detail、日期格式、空 what、非法 id）+ 重复 id + 缺字段 + 未知字段，全部被拦 |
| `safe_rules_never_under_personal_folders_and_keep_never_delete` | 展开后的每个 safe 模式都不在受保护目录；每条 keep 都没有 delete |
| `env_expansion_case_and_wildcards` | 全小写/全大写路径命中；`*` 匹配 Profile 名和 `thumbcache_*.db`；`**` 匹配飞书深层 CacheStorage 且比上层飞书规则优先；继承；`when_child` 有无 `Cache_Data` 的对照；docker_data.vhdx / WSL keep+migrate 且能腾出=0 |
| `missing_env_var_skips_pattern_but_keeps_rule` | 缺 `ProgramFiles(x86)` / `TEMP` 时只跳过相应模式，规则仍在，`%LOCALAPPDATA%\Temp` 兜底生效 |
| `tier_summary_splits_bytes_by_rule` | 构造树，safe/review/keep/未覆盖逐字节核对；self 容器不认领散文件 |
| `heuristic_signatures_on_real_dirs` | 在 `%TEMP%` 下**真实建目录**并用真实扫描器扫，12 个签名逐一命中（含 150 MB 模型文件、`set_len` 造）；没有 package.json 的 node_modules 不误判；未知项线索齐全；全部非 safe |
| `heuristic_sibling_marker_found_even_when_parent_files_aggregated` | 父目录 80 个文件使 package.json 被聚合进“其余小文件”，仍能识别 node_modules |
| `heuristic_registry_owner_and_age` | 假注册表：安装目录、名字、发布者三种归属；同一厂商 3 个软件时 `NVIDIA App` 精确归属、陌生子目录只说到厂商；`now` 推后 400 天出现“个月没动过” |
| `llm_request_body_has_no_file_content_and_redacted_path` | 文件里写机密串，序列化请求体里没有；`C:\Users\king` 已变 `~`；只有 9 个字段 |
| `llm_end_to_end_against_local_fake_server` | 起一个本地 TCP 假 OpenAI 服务，抓**真实发出的 HTTP 字节**：`POST /v1/chat/completions`、带 Bearer、不含机密串、用户名已脱敏、结果 review+Llm |
| `llm_network_failure_is_graceful` | 连 127.0.0.1:9 返回 Err 不 panic；未启用时给“去设置”提示 |

---

## 3. CLI 对真实 C 盘：`diskwhy-cli explain C:\ --top 50`

```
DiskWhy 解释：C:\
扫描 40.9 秒 | 卷已用 183.33 GB | 规则 179 条（含用户规则，加载+注册表共 15 ms）| 注册表已安装软件 260 个（读取 13 ms）| 解释前 50 大目录用时 10 ms
  #          占用  档位      来源                                  路径
  1   116.67 GB  保留      规则 users-root                       C:\Users
 10    11.99 GB  保留      规则 winsxs                           C:\Windows\WinSxS
 13    10.28 GB  需判断     规则 recycle-bin                      C:\$Recycle.Bin
 28     7.48 GB  保留      规则 docker-data-vhdx                 C:\Users\king\AppData\Local\Docker\wsl\disk
 32     6.30 GB  需判断     启发式 media-files                     C:\Users\king\AppData\ACLOS
 38     5.29 GB  保留      规则 wsl-distro-vhdx                  C:\Users\king\AppData\Local\wsl\{d1f9e0f8-…}
 40     4.93 GB  需判断     规则 feishu-larkshell                 C:\Users\king\AppData\Roaming\LarkShell
 45     4.53 GB  需判断     规则 nvidia-app-updates               C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts
 48     4.20 GB  可清理     规则 user-temp                        C:\Users\king\AppData\Local\Temp
 49     3.98 GB  需判断     规则 chrome-on-device-model           C:\Users\king\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel
 …（完整 50 行见 coverage.md）
统计（前 50 大目录）：规则命中 47（精确 34 + 继承上层 13）| 启发式 3 | 未知 0 | 规则+启发式覆盖率 100.0%
全盘按规则库归档（153 ms）：可清理(safe) 29.04 GB | 需判断(review) 60.89 GB | 保留(keep) 75.13 GB | 规则未覆盖 28.44 GB | 命中规则 93/179
```

`--top 200`：规则 172（精确 80 + 继承 92）| 启发式 19 | 未知 9 | 覆盖率 95.5%，解释 200 个目录用时 43 ms。未知项和下一批规则的待办见 coverage.md。

开发过程中实跑发现并修掉的两个问题（都补了测试）：
1. 注册表归属把 `ProgramData\NVIDIA Corporation\NVIDIA App` 说成了“NVIDIA Nsight”——厂商文件夹名 `NVIDIA` 前缀匹配到了第一个 NVIDIA 软件。改为：多层文件夹名先精确匹配，前缀匹配只接受唯一结果，否则只说到厂商。
2. `scope = "self"` 的容器规则在汇总和“其余小文件”上认领了容器里的散文件，已改为只说明文件夹本身。

---

## 4. GUI（`target\release\diskwhy.exe`，`npm run tauri build -- --no-bundle` 构建，普通权限）

截图方法同阶段 A：PowerShell 用 UI Automation 的 `InvokePattern` 点按钮/行、`ValuePattern` 填输入框、`ScrollPattern` 滚动详情面板，`PrintWindow(PW_RENDERFULLCONTENT)` 抓窗口。GUI 内扫描 C 盘用时 46.6 秒。

| 文件 | 内容 |
|---|---|
| `screenshots/phaseB-1-list-root.png` | 扫描完成：左上归属条，右上“本次扫描能腾多少”汇总卡（可放心清理 29.05 GB / 需要你判断 62.35 GB / 建议保留 75.13 GB / 规则还没覆盖 28.44 GB）；`C:\` 列表每行左侧档位色条 + 档位小标签 + 一句话“这是什么”；右侧 C 盘根目录的说明 |
| `screenshots/phaseB-2-list-and-safe-detail.png` | `AppData\Local` 列表（绿/黄/红混排）；选中 `Temp`：大字“可以放心清理” + 六项 + 该怎么腾 |
| `screenshots/phaseB-3-keep-migrate-docker.png` / `phaseB-3b-…-howto.png` | `Docker\wsl\disk\docker_data.vhdx`：“建议保留，别删”，能腾出“不能删；迁到别的盘可让 C 盘腾出约 7.48 GB”，该怎么腾里写 Docker Desktop 的 Disk image location 和 prune；来源 `规则库：docker-data-vhdx（分组 dev-caches）`，两个参考链接（带“复制链接”） |
| `screenshots/phaseB-4-unknown-clues.png` | `AppData\Local\OpenAI`：灰色“未知”，“我也不确定，这是我看到的线索”列出最大子项、扩展名分布（.exe 93%）、文件数、修改时间、在 AppData 下、注册表里找不到“OpenAI”；下面是“问 AI（先去设置里启用）”按钮和发送内容说明 |
| `screenshots/phaseB-5-settings-test-fail.png` | 从未知项的按钮进入设置页；点“填入本机 Ollama”后测试连接，本机 Ollama 返回 `HTTP 404 model 'qwen2.5:7b' not found`——**错误显示在对话框里，没有弹窗** |
| `screenshots/phaseB-6-settings-test-ok.png` | 模型改成本机已有的 `qwen3-vl:4b-instruct-q4_K_M` 并启用，测试连接：“连接成功，用时 7.1 秒，模型回复：你好” |
| `screenshots/phaseB-7-ask-ai-result.png` | 保存后对 `OpenAI` 点“问 AI 这是什么”（3.95 秒返回），黄色虚线框“AI 推测 · 需要你来判断”，可信度 80%，来源“AI 推测（模型 qwen3-vl:4b-instruct-q4_K_M）”。这是真实的本机模型回答（模型只看到名字和大小，猜得比较泛），也说明了为什么 AI 结果只能当 review |

截图结束后已删除测试时写入的 `%LOCALAPPDATA%\DiskWhy\settings.json`，恢复默认关闭。

---

## 5. 查过的官方文档

- Win32：`RegGetValueW`（`pcbData` 含结尾 NUL、`ERROR_MORE_DATA` 时返回所需大小、不带 `RRF_NOEXPAND` 时自动展开 `REG_EXPAND_SZ`）；“Accessing an Alternate Registry View”（用 `KEY_WOW64_64KEY` / `KEY_WOW64_32KEY` 分两遍枚举、子键要用同一个标志、`Wow6432Node` 是保留名不要直接用——实现里 HKLM 32 位那一遍就是用 `KEY_WOW64_32KEY` 打开同一路径，而不是拼 `WOW6432Node`）。
- ureq 3.4.2：docs.rs 首页 + 本地 crate 源码（`Agent::config_builder`、`timeout_global`、`http_status_as_error`、`TlsConfig::provider`、`native-tls` feature 定义、默认 provider 是 Rustls）。
- WSL basic-commands（export/import/unregister）、本机 `wsl --help`（`--manage --move`）、Ollama FAQ（`OLLAMA_MODELS`、默认路径）、Docker Desktop settings、DISM WinSxS 清理、powercfg 休眠、Delivery Optimization PowerShell cmdlet、conda clean / manage-environments、HF manage-cache 等（完整列表即规则里的 57 个 refs，全部 200）。
- Tauri 2：沿用阶段 A 查过的 calling-rust（`State`、async command 返回 `Result`）与 Channel 用法，本阶段没有新增插件和权限。

---

## 6. 没做到 / 没验证的

- **最后一次 `cargo test --workspace` 没能跑完 Tauri crate**：跑到一半 D 盘满了（只剩 0.14 GB，`os error 112`，同时报 C 盘页面文件太小），把 `target\debug` 里的若干 rlib 写坏了。我删掉了本项目自己的 `target\debug`（约 5.6 GB 可再生的编译缓存）后重跑了 core + cli 的全部测试（上面 47 个全绿）。Tauri crate 本身没有测试，它的代码在此之后没有改动，且 `npm run tauri build -- --no-bundle` 发布版构建成功、截图全部来自这个构建。**D 盘现在只剩约 2.4 GB，需要你清一下，否则下次构建还会失败。**
- 同上原因，没有在最后再跑一次整个工作区的 `cargo build --release`；发布版 CLI 和 GUI 是分别单独构建成功的。
- **参考链接不能点开**：WebView 里打开外部链接需要 `tauri-plugin-opener`，本机 cargo/npm 缓存里都没有，为不引入新的网络依赖，改成“复制链接”按钮。
- **86 条规则的路径本机不存在**，没在本机核实（见 1.3）；尤其国内软件（QQ 音乐、酷狗、钉钉、迅雷、B 站、企业微信、腾讯会议录像、百度网盘下载目录）的默认路径需要在装了这些软件的机器上复核。
- 飞书 `sdk_storage`、QQ 音乐 / 酷狗内部结构、罗技 G HUB、NVIDIA NGX、Claude `vm_bundles` 删了会怎样，规则里明确写了“不确定”。
- 注册表只读了当前用户的 HKCU；其他账户按用户安装的软件认不出来。以管理员身份的运行仍未验证（同阶段 A）。
- 启发式阈值（60% / 70% / 100 MB / 180 天）是按经验定的，只在本机和测试目录上验证过。
- AI 结果不保存，切换选中项就消失；API Key 明文存在本机 settings.json（设置页里写明了）。
- 前 50 覆盖率 100% 里有 15 个是容器规则、3 个是媒体文件启发式（见 coverage.md 的说明）；前 200 还有 9 个未知。
- 开发中踩到的坑：Bash 工具会把命令里的 `\\` 折成 `\`，导致早先写入的 `env.rs` / `pattern.rs` 里出现了非法的 `'\'` 字面量，已改正；之后所有含反斜杠的内容都改用文件编辑工具写入。

---

## 7. 决定（S22，2026-09-23 晚，用户在睡觉，自己拍板）

| # | 分岔 | 决定 | 理由 |
|---|---|---|---|
| 1 | 任务要求“先确认工作区干净，再 `checkout -b phase-b`”，但主工作区**不干净**：有另一轮会话 12:37–13:00 写下、没提交的阶段 D 代码（`src/action/`、`action_tests.rs`、清理抽屉 / 隔离区对话框等 10 个改动 + 7 个新文件） | 不碰它们。用 `git worktree add .worktrees/phase-b -b phase-b master` 在独立目录里开 phase-b 分支；`.worktrees/` 写进 `.git/info/exclude`（不是受版本控制的文件） | 直接切分支再 `git add -A` 会把别人没写完的删除功能一起提交进 phase-b，既不是本轮写的，也违反“本轮不实现删除动作”；worktree 让主工作区、master 分支和那批改动都原封不动 |
| 2 | 默认分支叫 `master` 不叫 `main` | 把“main 一字不改”理解为 master 一字不改 | 仓库里只有 master |
| 3 | 阶段 B 的大部分要求在 master 的 5c4c7e0（阶段 B）和 291bacd（阶段 C）里已经做完（规则库、Explanation、explain 入口、198 条规则） | 不重写，只补缺口：重复模式校验、“每条规则至少一个匹配样例”的测试、schema 合法写法测试、Defender 缓存规则、文档 | 重写只会制造回归；缺口逐条对照任务清单找出来 |
| 4 | 任务写的 `how: BuiltinTool(name)` | 保持现有写法：`how = "builtin_tool"` + 必填的 `how_detail` 写具体工具和命令 | 已有 200 条规则、GUI 和 CLI 都按这个结构；工具名单独成字段对用户没有额外信息，how_detail 里已写明（DISM、powercfg、磁盘清理、Set-MpPreference……） |
| 5 | “重复 match 报错”具体怎么算 | 原始写法重复（大小写、`/` 与 `\`、`%SystemRoot%` 与 `%WINDIR%` 视为相同）→ 报错，整条规则不加载；展开环境变量后和别的规则撞车 → 只跳过那个模式并警告；同一条规则里两种写法展开后相同（`%TEMP%` 与 `%LOCALAPPDATA%\Temp`）允许；`kind` / `when_child` 不同不算重复 | 原始重复是写规则的人的错，应该在测试里就炸；展开后撞车取决于某台机器的环境变量（比如 `%TEMP%` 被改成 `C:\Windows\Temp`），不能因此把整条规则扔掉 |
| 6 | “Defender 缓存”原来只有一条整体 keep 的规则 | 新增 `defender-scans`（keep + 系统工具）和 `defender-quarantine`（review + 系统工具），路径在本机核实存在；清理办法只写 Windows 安全中心界面和 `Set-MpPreference`，不教用户在资源管理器里删 | 两篇微软文档写的默认保留天数不一致（15 天 / 30 天；隔离区“不设置就一直留” / 组策略默认 90 天），规则里如实写了两种说法，让用户用 `Get-MpPreference` 看本机值 |
| 7 | 任务要求注册表“用 windows-registry 或 winreg crate，查 docs.rs 后再写” | **不换 crate**，继续用已经在依赖里的 `windows-sys` 直接调 `RegOpenKeyExW` / `RegEnumKeyExW` / `RegGetValueW` | 这段代码阶段 B 已写好、按 learn.microsoft.com 核过参数语义、本机读出 260 个软件（12–14 ms）；换成封装 crate 功能不变，却要多拉一个依赖并重测。任务真正要的“读 Uninstall 键做归属”已经满足 |
| 8 | “含 WOW6432Node”——HKCU 要不要也读一遍 32 位视图 | HKLM 读 64 位、32 位两个视图（`KEY_WOW64_64KEY` / `KEY_WOW64_32KEY`，不手拼 `WOW6432Node`）；**HKCU 只读一遍** | 微软文档 “Registry Keys Affected by WOW64” 的表里 `HKEY_CURRENT_USER\SOFTWARE` 在 Win7 及以后是 **Shared**（不重定向），没有单独的 32 位视图可读 |
| 9 | 启发式什么时候可以给 safe | 只有 `chromium-cache` 一个：文件夹名必须是 Chromium 的缓存目录名（Cache / Cache_Data / GPUCache / Code Cache 及其 js、wasm / GrShaderCache / DawnCache 等），**并且**里面每个看得到的文件都是磁盘缓存的标准文件名（`index`、`data_N`、`f_十六进制`、`16 位十六进制_0/_1/_s`、`index-dir\the-real-index`），并且不在个人文件夹下。名字对但混了别的文件 → `chromium-cache-mixed`（review） | “像缓存”不等于“是缓存”。标准文件名是 Chromium 磁盘缓存自己的格式，别的程序几乎不会这样起名；本机 Chrome 的 `Cache` 里还有新版才有的 `No_Vary_Search`（只含 .baj/.baf），也放行了 |
| 10 | LLM 请求体发什么 | 只发 4 个字段：`name`、`parent`（`X:\Users\<名字>` → `~`）、`children`（前 20 个子项的**名字**，不带大小）、`size`（总大小）。名字、父路径、子项名里出现的用户名（≥3 个字符）也换成 `~`。原来的扩展名分布、修改日期、文件数、子项大小都删了 | 任务原文“只含目录名、父路径脱敏、子项名摘要、大小”；多发的东西对猜测帮助不大，但会多泄露信息 |
| 11 | LLM 回复能决定什么 | 只收 `what / why / lose`；模型多给的 `tier`、`owner`、`confidence`、`how` 反序列化时直接丢掉。档位强制 review，可信度固定 `LLM_CONFIDENCE = 0.3`，`how` 固定“手动”，能腾出固定 0；归属只用注册表对得上的已安装软件，不让 AI 猜。每项最多 300 字、去掉控制字符 | “回复只能填 what/why/lose” |
| 12 | `explain()` 要不要自动问 LLM | 新增 `explain_full(target, ctx, Option<&dyn LlmOracle>)`：规则 → 启发式 → （传了 oracle **且**设置里开了开关）LLM → 未知。原来的 `explain()` 等于不传 oracle。列目录（每个子项都要解释）不传；只有用户点“问 AI”才走 LLM | 列目录时自动联网会让界面卡住、也会在用户不知情时发请求；把网络藏在 `LlmOracle` trait 后面，测试可以用假的 oracle 验证整条链，不联网 |
| 13 | 统一护栏放哪 | `enforce_guards()` 在 `explain` / `explain_full` / `llm::ask` 的出口都跑一遍：LLM、未知一律 review；启发式的 safe 只认白名单 `SAFE_HEURISTICS = ["chromium-cache"]`；任何来源的 safe 落在桌面 / 文档 / 下载 / OneDrive 下降为 review | 启发式内部已有一道，出口再兜一道，防止以后加新启发式时忘了 |
| 14 | node_modules 怎么说 | 有 package.json → 说出项目名和路径（node_modules 里面的包、嵌套的 node_modules 都指向最外层项目）；Electron 布局（`resources\app.asar.unpacked\node_modules`、`resources\app\node_modules`）或在 Program Files / `AppData\Local\Programs` / 注册表安装目录下 → `app-bundled-modules`，**keep**；都不是 → `node-modules-orphan`（review，手动） | 本机全量对照时 Typeless、Notion 自带的 node_modules 一开始被说成“遗留的，可以删”——删了软件就坏，所以加了这条 |
| 15 | 聊天数据换了位置怎么办 | 路径里有 `WeChat Files` / `xwechat_files` / `Tencent Files` / `WXWork` 这一层 → `chat-data`，keep + 迁移，而且排在所有签名最前面（视频占大头也不会被“媒体文件”抢走）；Program Files 下的 `WXWork` 是程序本体，排除 | 用户常把微信存储改到 D 盘（本机就是），规则库只认 `%USERPROFILE%\Documents\...` |
| 16 | conda 装在别处（本机是 `D:\conda`） | 里面有 conda-meta 且上一层叫 envs → `conda-env`（keep + 迁移，和规则 conda-envs 一致）；叫 envs 且旁边有 conda-meta → 同上；叫 pkgs 且有 `urls.txt` + `cache` → `conda-pkgs`（review + `conda clean --all`；规则里同一样东西是 safe，但启发式不许 safe） | 任务要求 conda envs 是 keep + Migrate |
| 17 | 游戏平台缓存第二批依据 | `epic-launcher-webcache`、`battlenet-cache` 标 safe，依据是官方客服文章；`steam-htmlcache` 标 review | Epic 页面直接抓取返回 403（有反爬），暴雪页面 WebFetch 断连、curl 跟随跳转后 200；这两篇的正文是通过搜索引擎对官方页面的摘录读到的（Epic：退出启动器后删 `%LOCALAPPDATA%\EpicGamesLauncher\Saved` 下的 webcache / webcache_4147 / webcache_4430；暴雪：结束 Agent.exe 后删 `%ProgramData%\Blizzard Entertainment`，不影响游戏数据）。Steam 的 htmlcache 没找到 Valve 的官方说明，按“拿不准就 review”处理 |
| 18 | “超过 180 天的缓存提示很久没用”对哪些生效 | 只对 category = cache 的（规则命中的缓存和启发式的缓存都算），写“很久没用了：已经 N 个月没动过……对应的软件可能已经不怎么用”；模型、程序等不写这句（模型很久不动是正常的），其余启发式仍然写原来的“已经 N 个月没动过了” | 规则命中时为了算这个要看子树的修改时间，最多看 5000 个节点，列目录仍然秒开 |
| 19 | 主工作区在我干活期间还在变（`TreemapView.vue` 新改了、多了 `phaseD-*.png` 截图） | 判断为另一轮会话正在主目录做阶段 D，继续只在 worktree 里改，不看、不碰主目录。两边共用 `target/`，cargo 有文件锁，会排队 | B-2 在 phase-b 上只改了 `DetailPanel.vue` 和 `SettingsDialog.vue` 各两行文案；阶段 D 也改了 `DetailPanel.vue`，将来合并时这两行可能要手动合 |
| 20 | （B-3）`explain --json` 输出什么 | 改成 **Explanation 数组**：每项是一条完整的 Explanation，另外平铺 `rank`（0 = 目标本身）、`path`、`allocated`，从 JSON 补读过的加 `reread: true`；`--json -` 写到标准输出（这时人看的表格改走标准错误）。原来 `--json` 写的“带统计的报告对象”挪到新参数 `--report` | 任务原文“--json 输出 Explanation 数组”；只有 Explanation 不带路径没法用，所以平铺三个定位字段，不另套一层 |
| 21 | （B-3）“直接对路径给出解释”怎么做 | `explain <路径>`：现扫这个路径，第 0 行解释它本身，后面是它下面前 N 大目录；`--top 0` 只解释它本身；路径是文件时扫它所在的文件夹再解释这个文件。统计只算第 1..N 行（`--top 0` 时算第 0 行） | 一个命令同时满足“解释一个路径”和“前 N 大逐个解释”；统计口径和 PLAN 的“前 50 大目录”一致，不把 `C:\` 本身算进去 |
| 22 | （B-3）只扫一个子目录时，上层规则怎么继承 | 新增 `view::outer_ancestors`：把扫描根以上每一层造成只带“直接子项名字和类型”的节点（读一次目录，不量大小、不递归），接在祖先链前面 | 否则只扫 `pip\cache\http-v2` 时认不出它在 `pip\cache` 规则里，只扫 `node_modules` 时也看不到旁边的 package.json；实测 `explain …\pip\cache\http-v2` 得到“pip-cache（继承上层）” |
| 23 | （B-3）scan 的 JSON 里树只到 3 层，前 50 大目录大多在更深处，`explain --from` 怎么办 | 导出时除了前 `--json-depth` 层，还保留**前 N 大目录连同它们的每一层上层**，这条链上每层的**直接子项**都保留（`view::export_tree`）。读回后前 N 大目录的集合和现扫一致 | 本机 C 盘 JSON 24 MB、24,186 个目录节点，比全量快照解压后小得多；链上的兄弟子项保留下来，规则的 `when_child` 和“旁边有没有 package.json”还能判断 |
| 24 | （B-3）JSON 被截断后，启发式看不到深处的文件怎么办 | 规则只看路径，截断不影响；**规则没命中、子树又不完整时**（`view::subtree_complete`：`dirs` / `files` 计数对不上），回磁盘把这个文件夹只读地重扫一遍再解释，行上标 `reread`，notes 里写明；`--no-reread` 可以关掉 | 测试里构造了“日志藏在第 3 层”的例子：不补读时认不出（未知），补读后是 `logs-dumps`，和现扫一致；本机 C 盘前 50 只补读了 2 个文件夹（215 ms） |
| 25 | （B-3）scan JSON 里每个目录节点的“解释摘要”在哪算 | 在**完整的**内存树上算（`ExplainSummary`：tier / owner / source / what），不调 LLM；`top_dirs` 里带完整 Explanation；另加 `explain` 字段写规则数、警告、用时。`--no-explain` 可以不附 | 摘要在截断前算，所以就算 JSON 里子树不全，摘要也是准的；本机 24,186 个节点 224 ms。读回 JSON 时 `explanation` 字段被忽略，旧的 `--from` 流程不受影响 |
| 26 | （B-3）`--llm` 什么时候真的联网 | 两道开关：命令行 `--llm` **并且**设置里已启用（`enabled` + 地址 + 模型）。只对“规则、启发式都认不出”的行问，而且在回磁盘补读之后才判断；设置没启用时只打印“本次不会联网” | 默认关闭；避免一个开关就把几百个目录名发出去；测试用本机假服务器核对：不加 `--llm` 请求数 0、规则认得的目录请求数 0、未知目录正好 1 次且请求里没有文件内容 |
| 27 | （B-3）覆盖率怎么算 | “规则 + 启发式”占前 N 大目录的比例，容器规则（`scope = self`）算命中，但单独列出个数，coverage.md 同时给“不算容器”的数字 | 容器规则确实回答了“这是什么”，但回答偏泛；两个数都给，读者自己判断 |
| 28 | （B-3）真实 C 盘第一轮发现：商店应用沙箱里的东西被说成“Claude 的数据” | 新增 `explain::msix_redirect`：路径在 `%LOCALAPPDATA%\Packages\<包>\LocalCache\{Local,Roaming,LocalLow}\<至少一层>` 时，按应用以为的原位置找规则（精确 → 上层 tree 规则 → 启发式），都认不出才退回包自己的规则并加提醒；真实路径上本来就有更具体规则的（如 `claude-desktop-vm`）照旧 | 依据 learn.microsoft.com《Understanding how packaged desktop apps run on Windows》：打包应用在 AppData 里**新建**的文件和文件夹写到每用户每包的私有位置、运行时合并显示在原位置。本机 Claude 桌面版的 `LocalCache\Local` 里有 pip、GitHub CLI、playwright 等别的程序的数据 |
| 29 | （B-3）`CARGO_TARGET_DIR` 放在别处的 Rust 编译目录旁边没有 Cargo.toml | `rust-target` 启发式多认一种：目录里同时有 `CACHEDIR.TAG` 和 `.rustc_info.json`（本身或上层 8 层内）。只查文件在不在，不读内容；只有 `CACHEDIR.TAG` 不算 | `CACHEDIR.TAG` 是 cargo 从 PR #8378 起在 target 目录放的备份排除标记（别的软件也会放，所以要两个一起）；`.rustc_info.json` 在 cargo 官方文档里没找到说明，是本机两个 target 目录里都看到的，**属于本机观察** |
| 30 | （B-3）前 50 已达 100%，还要不要补规则 | 不补。“未命中”的待办写进 coverage.md；需要查官方资料的两条（Raspberry Pi Imager 缓存、`AppData\Local\OpenAI\Codex`）标“要查”留给下一批 | 任务说“达不到才补规则”；没查证就写规则违反“先查文档” |
| 31 | （B-3）任务写“`cargo build --release`” | 只构建了 `-p diskwhy-cli` 的发布版，Tauri 壳用 `cargo check -p diskwhy` 确认能编译 | B-3 没改界面；worktree 里没有前端依赖，整个工作区发布版会连 Tauri 一起 LTO 编译，时间长而且和另一轮会话抢同一个 target/ |
| 32 | （B-3）第二轮要不要重扫 | 重扫（60 秒），不用第一轮的 JSON | 第一轮 JSON 里的摘要是修改前的引擎算的，重扫才能让“JSON 摘要”和“explain 结果”逐行对得上（实测 51/51 一致） |
| 33 | （B-3）coverage.md 怎么处理旧版本 | 整个重写（旧版在 master 的 5c4c7e0 里），路径里的用户名换成 `<用户>`（`--md` 输出自动做） | 旧版是 179 条规则时的数，而且没脱敏 |

## 8. B-1：规则库骨架 + 首批规则（S22 补做，分支 `phase-b`）

### 8.1 做了什么

对照任务清单逐条核对，已有的直接复用，缺的补上：

| 任务要求 | 状态 | 位置 |
|---|---|---|
| TOML schema：id / owner / category / tier / match / what / why / lose / how / refs / verified_on | 已有（另有 how_detail、scope、kind、when_child、owner_from_path） | `crates/diskwhy-core/src/explain/rules.rs` |
| `%LOCALAPPDATA%` `%APPDATA%` `%PROGRAMDATA%` `%USERPROFILE%` `%TEMP%` 等展开，`*` 通配，大小写不敏感 | 已有（白名单 11 个变量；另支持 `?` 和 `**`） | `explain/env.rs`、`explain/pattern.rs` |
| rules/ 目录嵌入 + 运行时加载用户目录 | 已有（`include_str!` 嵌入 6 个文件；`%LOCALAPPDATA%\DiskWhy\rules\*.toml`） | `rules.rs` 的 `EMBEDDED`、`load_user_dir` |
| 启动校验：根目录白名单、safe 不指向文档/桌面/图片/OneDrive、id 唯一、tier 合法 | 已有 | `rules.rs` 的 `validate` |
| 启动校验：**重复 match 报错** | **本轮新增** | `rules.rs` 的 `pattern_key` / `duplicate_check`；内置规则和用户规则都查 |
| Explanation（tier、owner、category、what、why、lose、gain_bytes、how、source、confidence）+ `explain(node)` | 已有（source = Rule{id} / Heuristic{name} / Llm{model} / Unknown） | `explain/mod.rs` |
| ≥ 40 条规则，Windows 系统 / 浏览器 / 开发缓存三组 | 已有 198 条，本轮 +2 条 Defender、修 1 条 → **200 条** | `rules/*.toml` |
| 单元测试：schema 合法/非法、展开、通配、safe 护栏 | 非法写法已有；**合法写法、非法枚举值、safe 护栏的 review 对照为本轮新增** | `tests/explain_tests.rs` |
| 单元测试：**每条规则至少一个匹配样例** | **本轮新增**（自动按模式造路径，并要求这条规则在样例上“胜出”） | `every_rule_has_a_matching_sample` |

**新查重立刻抓到一个真问题**：`pip-cache` 的 match 里写了 `%LOCALAPPDATA%\pip\cache` 和 `%LOCALAPPDATA%\pip\Cache`——匹配本来就不分大小写，第二个是多余的。已删掉一个。

**规则库现状（200 条）：**

| 文件 | 条数 | safe | review | keep |
|---|---:|---:|---:|---:|
| windows-system.toml | 58 | 10 | 7 | 41 |
| browsers.toml | 19 | 9 | 1 | 9 |
| dev-caches.toml | 41 | 16 | 12 | 13 |
| ai-models.toml | 13 | 0 | 12 | 1 |
| cn-apps.toml | 36 | 7 | 12 | 17 |
| misc.toml | 33 | 6 | 18 | 9 |
| **合计** | **200** | 48 | 62 | 90 |

任务点名的条目全部有规则，并由新测试 `b1_listed_items_real_paths` 用真实长相的路径逐条核对（41 个路径）：WinSxS→keep+DISM、Windows.old→review+磁盘清理、Installer→keep、传递优化→safe、hiberfil→review+powercfg、pagefile→keep、两个 Temp→safe、Prefetch→keep、缩略图→safe、`SoftwareDistribution\Download`→safe、`Windows\Logs`→review、Defender 扫描记录→keep / 隔离区→review；Chrome / Edge / Firefox / Brave 的 Cache、Code Cache、GPUCache、Service Worker 与通用 Chromium 缓存；npm / pnpm / yarn / pip / uv / cargo registry / go / gradle / maven / nuget / conda pkgs / Docker vhdx / WSL vhdx / VS / JetBrains / VS Code / Cursor。

### 8.2 怎么验证的

- `cargo test -p diskwhy-core`（在 worktree 里跑，`CARGO_TARGET_DIR` 指回主目录的 `target/`，D 盘）：**59 个通过，0 失败，1 个忽略**（忽略的是阶段 C 就有的 dedup 手动对照测试，要设 `DISKWHY_CMP_DIR` 才跑）。其中 explain_tests 从 22 个变成 26 个，新增 4 个：

| 测试 | 测什么 |
|---|---|
| `validation_accepts_legal_variants` | 4 种合法写法都能加载（how 数组、scope/kind/when_child/owner_from_path、全部根变量、safe 放在 `%USERPROFILE%\.cache`）；非法 tier / category / how / 日期都加载失败；5 个个人文件夹位置写成 safe 被拦、同样位置写成 review 放行 |
| `duplicate_match_is_rejected` | 8 种情况：跨规则重复（大小写 + 斜杠不同）、同规则写两遍、`%WINDIR%` = `%SystemRoot%`、when_child 不同不算重复、展开后撞车只跳过那个模式且规则照常生效、同规则互为兜底允许、用户规则与内置 `npm-cache` 重复被拒、`%TEMP%` 被改到 `C:\Windows\Temp` 时 user-temp 仍然生效 |
| `every_rule_has_a_matching_sample` | 对 200 条规则的 409 个模式，把 `*` / `?` / `**` 换成具体名字造出 **435 个样例路径**，每条规则至少要在一个样例上被选为说明（顺带抓出被更具体规则完全盖住的死规则——当前没有），并核对 what / category / 可信度来自该规则 |
| `b1_listed_items_real_paths` | 上面 41 个真实长相的路径：规则 id、档位、文案非空、keep 不给删除且能腾出 = 0、safe 能腾出 = 实际大小；Defender 两条规则写了 `Set-MpPreference` 的参数名 |

- 真实环境：`diskwhy-cli explain C:\Users\king\AppData\Local\pip --top 5` 用本机真实环境变量加载规则，**没有任何规则警告**，`pip\cache` 174 MB 由 `pip-cache` 说明为“可清理”（204 条 = 内置 200 + 本机 `%LOCALAPPDATA%\DiskWhy\rules\phaseD-demo.toml` 里另一轮会话留下的 4 条演示规则，没动它）。
- 参考链接：规则里 65 个不重复的 `refs` 全部用 curl 请求，**65 个都返回 200**。新加的 3 个微软文档用 WebFetch 读过原文：`Set-MpPreference` 里 `ScanPurgeItemsAfterDelay`（默认 15 天，0 = 不删）和 `QuarantinePurgeItemsAfterDelay`（0 或不设 = 一直保留）；“Configure remediation” 页的组策略表写默认扫描历史 30 天、隔离区 90 天；“Restore quarantined files” 页写了 Windows 安全中心 → 保护历史记录的操作路径。
- 本机核实：`C:\ProgramData\Microsoft\Windows Defender\` 下确实有 `Scans\` 和 `Quarantine\`，普通权限打开 `Scans` 报“Permission denied”——这也是规则里写“手动删不了”的依据。

### 8.3 没做 / 没验证的

- 没有在 GUI 里重新截图（B-1 只动了规则库和测试，界面代码没改；GUI 构建和截图留给后面的小节）。
- 没跑 `cargo build --release` 和 `npm run tauri build`：B-1 只改了 core 的规则加载和规则文件，公开接口没变；CLI 已用 `cargo run -p diskwhy-cli` 编译运行过。
- `defender-quarantine` 的“删除”操作路径来自微软文档里“还原”那一节的同一个界面（保护历史记录），界面上的“删除”按钮本身**没有在本机点过**（本机隔离区是空的，也不该为了测试去制造隔离项）。
- 第 6 节里说的“86 条规则的路径本机不存在”仍然成立，本轮没有新的机器可以核实。
- 主工作区里那批没提交的阶段 D 代码没看、没测、没动；它和 phase-b 分支将来合并时，两边都改了 `rules/` 之外的文件的可能性很小（phase-b 只改了 `explain/rules.rs`、`explain_tests.rs`、两个规则文件和本文档），但合并前仍要跑一遍全部测试。

### 8.4 下一步从哪接

- 分支 `phase-b` 在 `D:\desk\claudex\13\diskwhy\.worktrees\phase-b`（git worktree）。后续小节继续在这个目录里做、在这个分支上提交；**不要**在主目录 `checkout phase-b`（git 会拒绝，因为 worktree 占着它；而且主目录有别人的未提交改动）。构建时 `CARGO_TARGET_DIR=D:/desk/claudex/13/diskwhy/target`，复用已编译的依赖。
- 用户醒来后要决定：主目录那批阶段 D 改动是继续做完提交，还是先放一边；之后再把 `phase-b` 合回 master（`git merge phase-b`，预计无冲突）。
- 规则库下一批：coverage.md 里前 200 的 9 个未知项、以及在装了国内软件的机器上核对那 86 条没命中的路径。

## 9. B-2：第二批规则 + 启发式 + 归属推断 + LLM 兜底（S22，分支 `phase-b`）

### 9.1 做了什么

先对照任务逐条核：master 上的阶段 B 已经有 200 条规则（任务点名的 AI 模型缓存、国内软件、conda envs、Steam shadercache 全都在）、12 个启发式、注册表快照和默认关闭的 LLM。B-2 只补真正缺的和做得不对的：

| 任务要求 | 状态 | 位置 |
|---|---|---|
| 规则总数 ≥ 80，AI 模型 / 国内软件 / conda envs / 游戏平台 | 已有 200 条；本轮 +3 条游戏平台（`steam-htmlcache` review、`epic-launcher-webcache` safe、`battlenet-cache` safe）→ **203 条** | `rules/misc.toml` |
| 微信、QQ 聊天文件与 FileStorage 必须 keep + Migrate | 已有（`wechat-files` / `qq-files` / `wecom-files`，FileStorage 按 tree 继承）；本轮加测试逐个路径核对；另加启发式 `chat-data` 兜住改过位置的聊天数据 | `tests/b2_tests.rs`、`explain/heuristics.rs` |
| 启发式：Chromium 缓存 → safe | **本轮改**：原来一律 review。现在签名严格才 safe，不严格的叫 `chromium-cache-mixed`（review） | `heuristics.rs` 的 `strict_chromium_cache` / `is_chromium_disk_cache` |
| site-packages → review；node_modules → review 并提示项目；target + Cargo.toml → review | 已有；**本轮加**：node_modules 说出项目名和路径、node_modules / target / site-packages 里面的子文件夹也能认出（原来只认最外层）、没有 package.json 的叫 `node-modules-orphan`、软件自带的叫 `app-bundled-modules`（keep） | `heuristics.rs` |
| 模型权重占多数 → review；*.vhdx → keep；*.log 占多数 → review | 已有（阈值见 1.4 节） | — |
| 超过 180 天的缓存提示“很久没用” | **本轮加**：缓存类（规则和启发式都算）写“很久没用了：……” | `heuristics::long_unused_note`、`mod.rs` 的 `from_rule` |
| 注册表 Uninstall（HKLM/HKCU，含 WOW6432Node）的 DisplayName / InstallLocation / DisplayIcon，按目录名或路径前缀匹配 | 已有（决定 7、8）；本轮补测试 | `explain/registry.rs`、`heuristics::owner_from_path` |
| LLM：OpenAI 兼容端点、支持本机 Ollama、默认 enabled=false | 已有 | `explain/llm.rs` |
| LLM 请求体只含目录名、脱敏父路径、前 20 个子项名、大小 | **本轮改**：从 9 个字段收紧到 4 个（决定 10） | `llm::LlmInput`、`build_input_with` |
| LLM 回复只填 what/why/lose，tier 强制 review、source=Llm、可信度低 | **本轮改**（决定 11） | `llm::LlmAnswer`、`to_explanation`、`LLM_CONFIDENCE` |
| 用 mock HTTP 或纯函数测试脱敏和字段约束，不真的联网 | **本轮加** `LlmOracle` trait + 测试里的 `MockOracle`（纯函数）；原有的本地假服务器测试（127.0.0.1）保留 | `llm.rs`、`tests/b2_tests.rs` |
| `explain()` 串起来：规则 → 启发式 → （可选）LLM → Unknown，记录 source / confidence | **本轮加** `explain_full` + `enforce_guards`（决定 12、13） | `explain/mod.rs` |
| 测试覆盖优先级与护栏（启发式只有 Chromium 缓存能 safe，LLM 永不 safe） | **本轮加** | `tests/b2_tests.rs` |

另外：conda 装在别处时的 `conda-env`（keep + 迁移）和 `conda-pkgs`（review + `conda clean --all`）两个启发式（决定 16）；GUI 详情面板和设置页里“会发送什么”的说明改成新的四样东西，“属于”一栏在 AI 结果里改为“没查到（归属只看注册表，不让 AI 猜）”。

**规则库现状（203 条）：**

| 文件 | 条数 | safe | review | keep |
|---|---:|---:|---:|---:|
| windows-system.toml | 58 | 10 | 7 | 41 |
| browsers.toml | 19 | 9 | 1 | 9 |
| dev-caches.toml | 41 | 16 | 12 | 13 |
| ai-models.toml | 13 | 0 | 12 | 1 |
| cn-apps.toml | 36 | 7 | 12 | 17 |
| misc.toml | 36 | 8 | 19 | 9 |
| **合计** | **203** | 50 | 63 | 90 |

**启发式现状（按尝试顺序）**：chat-data（keep）→ chromium-cache（**唯一可 safe**）/ chromium-cache-mixed → conda-pkgs / conda-env（keep）/ python-venv / python-env → site-packages 里面（python-env）→ app-bundled-modules（keep）/ node-modules / node-modules-orphan → rust-target → dotnet-build → model-weights → virtual-disk（keep）→ logs-dumps → downloaded-installer → temp-files → media-files → 注册表归属（registry-install-dir / registry-program-dir / registry-appdata）→ 未知。

### 9.2 怎么验证的

**单元 / 集成测试**（`CARGO_TARGET_DIR` 指回主目录 `target/`）：`cargo test -p diskwhy-core -p diskwhy-cli` → **70 个通过，0 失败，2 个忽略**（忽略的是阶段 C 的 dedup 手动对照和本轮的全量对照，都要设环境变量才跑）。B-1 时是 59 个，本轮新增 `tests/b2_tests.rs` 11 个：

| 测试 | 测什么 |
|---|---|
| `b2_rules_total_and_listed_software` | 规则 ≥ 80；任务点名的 27 条规则逐条核档位；8 类 AI 模型都有 migrate + delete；聊天文件和 conda envs 都是 migrate 且没有 delete |
| `b2_listed_items_real_paths` | 31 个真实长相的路径（HF hub/transformers、Ollama、torch、ModelScope、insightface、paddle、LM Studio、whisper、微信 FileStorage\File / Video、xwechat_files、QQ FileRecv / nt_qq、企业微信、飞书、钉钉、网易云、QQ 音乐、酷狗、B 站、WPS、搜狗、微软拼音、conda envs、Steam shadercache / htmlcache、Epic webcache 两种名字、暴雪缓存）：规则 id、档位、文案非空、keep 不给删除且能腾出 = 0 |
| `b2_chromium_cache_strict_signature_is_the_only_safe_heuristic` | 真实建目录：新版 Cache\Cache_Data（含 No_Vary_Search）、GPUCache、Code Cache 及 js → safe 且带“很久没用”；混了 .docx / readme.txt 的、名字不对的 → mixed（review）；“文档”里的 → review 并说明原因；最后把整棵树每个文件夹都解释一遍，safe 只能来自 chromium-cache |
| `b2_heuristics_project_hints_chat_data_conda_and_inheritance` | node_modules 本身 / 里面的包 / 嵌套的都指向项目「web」；遗留的；Typeless 式 `app.asar.unpacked\node_modules` 和 `resources\app\node_modules` → keep；target\debug → 项目「rs」；site-packages\numpy；改了位置的 `WeChat Files`（视频占大头）→ keep + 只给迁移；Program Files 下的 WXWork 不算；conda envs / 某个环境 → keep；conda pkgs → review + 系统工具 |
| `b2_long_unused_note_on_cache_rules` | 规则命中的 pip 缓存 400 天没动 → “很久没用”；HF 模型不写；10 天不写 |
| `b2_registry_owner_by_install_location_prefix_and_name` | InstallLocation 为空时用 DisplayIcon 的 exe 目录做路径前缀匹配；AppData 下文件夹名匹配 DisplayName |
| `b2_real_registry_reads_three_views` | 真实注册表：来源只有 HKLM64 / HKLM32 / HKCU 三种 |
| `b2_llm_input_four_fields_top20_and_username_redacted` | 序列化后只有 `children / name / parent / size` 四个键；32 个子项只发 20 个；`bobtools` → `~tools`、`Bob-Notes.txt` → `~-Notes.txt`、叫 `BOB` 的子文件夹 → `~\`；整个请求体里找不到 “bob”；扫用户目录本身时名字是 `~`；用户目录不在 `C:\Users` 下时靠调用方给的用户名脱敏 |
| `b2_llm_answer_only_what_why_lose` | 回复里带 `tier:"safe"`、`owner`、`confidence:0.99`、`how:["delete"]` 全部不算数；控制字符被去掉；1000 字的字段截到 300 |
| `b2_explain_chain_priority_rule_heuristic_llm_unknown` | 用假 oracle（纯函数，记录调用次数和请求体）：规则命中不问、启发式命中不问、都没认出来且开关打开才问一次（结果 review / Llm / 0.3 / 不写 AI 猜的归属 / 保留线索 / 请求体没有用户名）；开关默认关闭时一次都不调用；不传 oracle 等于 `explain()`；oracle 出错或回复不是 JSON → 退回未知并在 notes 里写原因 |
| `b2_enforce_guards_on_every_source` | 直接构造各来源的解释过护栏：LLM 的 safe / keep → review 且能腾出清零；未知 → review；非白名单启发式的 safe → review；chromium-cache 在普通位置保持 safe、在“文档”里 → review；规则的 safe 在“文档”里 → review，keep 不动 |

旧测试按新行为改了 4 处：Chromium 缓存样例从 review 改为 safe、没有 package.json 的 node_modules 从“不认”改为 `node-modules-orphan`、LLM 字段数从 9 改为 4 个具名字段、AI 可信度从“≤0.9”改为固定 0.3。

**变异检查**：临时把启发式出口的 safe 白名单和 `enforce_guards` 里的启发式分支都拿掉，`b2_chromium_cache_…` 和 `b2_enforce_guards_…` 两个测试立刻失败；恢复后全绿。说明护栏测试真的在看护栏。

**本机全量对照**（手动测试 `b2_manual_real_dir_sweep`，发布版，本机真实环境变量 + 内置规则 + 本机 `%LOCALAPPDATA%\DiskWhy\rules` 里另一轮会话留下的 4 条演示规则 = 207 条、真实注册表 260 个软件，只读扫描、不联网）：

| 目录 | 扫描 | 文件夹数 | 解释用时 | 结果要点 |
|---|---:|---:|---:|---|
| `C:\Users\king\AppData\Local` | 31 s | 73,148 | 1.9 s | 规则精确 970 / 继承 63,215；启发式判 safe 的只有 1 处（Playwright 浏览器配置里的 `Shared Dictionary\cache`，264 KB）；`app-bundled-modules` 26 处（Typeless、Notion 自带依赖，keep）；未知 3,797 个文件夹 21.1 GB |
| `D:\conda` | 16 s | 89,246 | 6.0 s | 16 个环境全部 `conda-env`（keep，合计 49.0 GB）；`D:\conda\pkgs` 1.44 GB → `conda-pkgs`（review + conda clean） |
| `D:\desk` | 6 s | 72,522 | 5.9 s | 启发式 safe 283 处共 1019 MB，逐条看都是测试用 Chrome 配置目录里的 `Default\Cache`；`rust-target` 895 个（含本项目 `target\`）；`node-modules` 5,698 个、`node-modules-orphan` 119 个（多是 `dist\client\node_modules` 这类打包输出） |

对照中发现并修掉的问题：**Typeless、Notion 安装目录里的 `resources\app.asar.unpacked\node_modules` 第一次跑时被说成“遗留的依赖，可以删”**——删了软件就坏。改成先判断是否是软件自带（Electron 布局 / 程序目录 / 注册表安装目录）→ keep，并补了测试。另外 kimi-code 在 AppData 里的 node_modules 没有 package.json，原文案“项目可能已经删了”不对，改成“在 AppData 里通常是某个软件运行要用的”，办法改成手动。

**其他**：`cargo check -p diskwhy`（Tauri 壳）通过；两个新参考链接：暴雪文章 curl 跟随跳转后 200（规则里改成跳转后的地址 `…/en/help/article/34721`），Epic 文章返回 403（站点反爬，浏览器能打开）。

### 9.3 没做 / 没验证的

- **没有在 GUI 里重新构建和截图**：worktree 里没有 `node_modules`，而主目录正被另一轮会话用来做阶段 D；本轮前端只改了两处文案，Tauri 壳 `cargo check` 通过，但 `npm run tauri build` 和截图没做。
- **LLM 没有连真实模型重测**：新的四字段请求体只用假 oracle 和本地假服务器验证过；本机 Ollama 能不能靠这么少的信息猜得像样，没试（按要求不联网）。可以预期猜得比原来更泛，这也是可信度固定 0.3 的原因。
- Epic、暴雪两条规则的依据是官方客服文章经搜索引擎摘录的正文，不是我直接打开页面读到的（Epic 403、暴雪 WebFetch 断连）；本机没装这两个启动器，路径没在本机核实。Steam htmlcache 本机存在（301 MB），但没找到官方说明，所以是 review。
- Chromium 严格签名在本机只见过 Chrome / Edge / Steam 内置浏览器 / Playwright 配置的缓存格式；别的 Electron 程序如果在缓存目录里放了额外文件，会落到 `chromium-cache-mixed`（review），这是有意偏保守。
- 被合并进“其余小文件”的文件看不到名字：Chromium 缓存判断只能核对最大的 50 个文件名，`index` 被合并掉时去文件系统确认它存在。理论上有漏网的可能，概率很低。
- 注册表仍没换成 windows-registry / winreg crate（决定 7）；以管理员身份运行、其他用户账户的 HKCU 仍未验证。
- 第 6 节里“86 条规则的路径本机不存在、没在装了这些软件的机器上核实”仍然成立。

### 9.4 下一步从哪接

- 继续在 `D:\desk\claudex\13\diskwhy\.worktrees\phase-b` 做，提交到 `phase-b`，构建用 `CARGO_TARGET_DIR=D:/desk/claudex/13/diskwhy/target`。
- 用户醒来后要决定：主目录里那批阶段 D 改动怎么处理；之后 `git merge phase-b`。`src/components/DetailPanel.vue` 两边都改过，合并时注意那两行“发送什么”的文案要保留 B-2 的版本。
- 想看真实模型在四字段下的表现：在设置里填本机 Ollama，对 `AppData\Local` 里的未知项点“问 AI”，把结果和第 4 节的截图对比。
- 手动全量对照可以随时重跑：`DISKWHY_SWEEP_DIR=<目录> cargo test --release -p diskwhy-core --test b2_tests b2_manual_real_dir_sweep -- --ignored --nocapture`。

## 10. B-3：CLI explain + 真实 C 盘覆盖率（S22，分支 `phase-b`）

### 10.1 做了什么

| 任务要求 | 状态 | 位置 |
|---|---|---|
| `explain` 子命令：对 scan 结果 JSON 解释 | **本轮加**：`explain --from scan.json [<路径>]`；JSON 截断时规则没命中的文件夹回磁盘只读补读（决定 23、24） | `crates/diskwhy-cli/src/explain_cmd.rs` |
| `explain` 直接对路径解释 | **本轮加**：`explain <路径>`（文件也行），第 0 行是它本身，`--top 0` 只解释它；扫描根以上的规则继承靠 `outer_ancestors`（决定 21、22） | `main.rs`、`view::outer_ancestors` |
| `--json` 输出 Explanation 数组 | **本轮改**（决定 20）；原来的报告挪到 `--report`；`--md` 输出 coverage.md 的表格（用户名脱敏） | `explain_cmd.rs` |
| `--top N` 对前 N 大目录逐个解释 | 已有，改为基于目标文件夹（不一定是扫描根） | 同上 |
| `--llm` 开关，默认关 | **本轮加**，要命令行和设置两道开关都开，只问认不出的（决定 26） | 同上，复用 B-2 的 `explain_full` / `HttpOracle` |
| scan 的 JSON 里每个目录节点附解释摘要（tier / owner / source / 一句话） | **本轮加** `ExplainSummary`、`view::export_tree`；`top_dirs` 带完整解释；前 N 大目录连同上层保留在树里（决定 23、25） | `explain/mod.rs`、`view.rs`、`main.rs` 的 `write_scan_json` |
| 真实 C 盘 scan + explain --top 50，写 coverage.md | 做了两轮（第二轮是修完说错的地方后重扫），见 [coverage.md](coverage.md) | `docs/coverage.md` |
| 集成测试：临时目录假树跑 explain | **本轮加** CLI 端到端 3 个 + core 3 个 | `crates/diskwhy-cli/tests/explain_cli.rs`、`crates/diskwhy-core/tests/b3_tests.rs` |

另外，真实 C 盘第一轮暴露的两处引擎问题一并修了（决定 28、29）：商店应用沙箱重定向（`explain::msix_redirect`，新 `RuleSet::best_tree_exact`）、单独放置的 cargo 编译目录（`heuristics::cargo_target_dir`）。

**用法：**

```
diskwhy-cli scan C:\ --top 50 --json scan.json                 # 树里每个目录带摘要，前 50 大目录连同上层都保留
diskwhy-cli explain --from scan.json --top 50 --json out.json    # 不重扫；out.json 是 Explanation 数组（51 项，第 0 项是 C:\）
diskwhy-cli explain --from scan.json --top 50 --report r.json --md coverage.md
diskwhy-cli explain C:\Users\me\AppData\Local\pip --top 0 --json -   # 只解释这一个路径，JSON 写到标准输出
diskwhy-cli explain D:\somewhere --top 20 --llm                  # 设置里启用了 AI 才会问，只问认不出的
```

### 10.2 真实 C 盘结果（详见 coverage.md）

| 范围 | 规则 | 启发式 | 未知 | 规则 + 启发式 |
|---|---:|---:|---:|---:|
| 前 50 大目录 | 47（94.0%；精确 35 其中容器 14，继承 12） | 3（6.0%） | 0（0.0%） | **100.0%**（目标 ≥ 80%）；不算容器 72% |
| 前 200 大目录 | 183（91.5%） | 16（8.0%） | 1（0.5%） | 99.5% |

- 扫描 60.1 秒（普通权限；同时另一轮会话在编译，比平时的 41 秒慢）；`scan.json` 24 MB，24,186 个目录节点全部带摘要（224 ms）；`explain --from` 51 行 219 ms，其中补读 2 个文件夹 215 ms。
- 用脚本逐行核对：`explain` 的 51 行和 `scan.json` 里对应节点的摘要 **51/51 一致**；`top_dirs` 50 项都带完整解释。
- **第一轮的 100% 里有 4 行是说错的**（Claude 商店应用沙箱里的 cargo 编译目录被说成“Claude 的登录信息和设置”），修完后第二轮其中 3 行变成 `rust-target`，1 行保留包规则但带提醒。只看覆盖率会高估质量，这次靠逐行人工看才发现。

### 10.3 怎么验证的

- `cargo test -p diskwhy-core -p diskwhy-cli`（`CARGO_TARGET_DIR` 指回主目录 `target/`）：**76 个通过，0 失败，2 个忽略**（忽略的两个同 B-2，要设环境变量才跑）。B-2 时是 70 个，本轮新增 6 个：

| 测试 | 测什么 |
|---|---|
| `b3_scan_json_carries_summaries_and_explain_from_json_matches_live`（CLI） | 真跑 `diskwhy-cli.exe` 子进程，`%LOCALAPPDATA%` 等环境变量指到假树里：`scan --json --json-depth 1` 后树里每个目录都有摘要（且只是摘要，没有 lose）、第 5 层的前 N 大目录也在；`explain --from` 与现扫逐行一致（tier / source / what）；pip 精确 + 继承、Temp、node_modules 说出项目「proj」、严格 Chromium 缓存 safe、第 3 层的日志、未知项带线索；safe 只来自规则或 chromium-cache；只保留前 3 大目录的 JSON 里 `logsdir` 回磁盘补读后认出 `logs-dumps`，`--no-reread` 时认不出；`--from` 加子路径；`--report` 各来源个数之和 = 总数、不含第 0 行；`--md` 表头和待办小节 |
| `b3_explain_single_paths_and_files`（CLI） | `--top 0` 只出一行；只扫 `node_modules` 也能看到上层的 package.json；只扫 `pip\cache\http-v2` 认出继承的 `pip-cache`，`inherited_from` 指向 `pip\cache`；解释单个文件；路径不存在时失败退出不崩；`scan --llm` 被拒 |
| `b3_llm_flag_is_off_by_default_and_only_asks_for_unknowns`（CLI） | 本机 127.0.0.1 假服务器：设置里启用但不加 `--llm` → 0 次请求；加 `--llm` 解释规则认得的目录 → 0 次；未知目录 → 正好 1 次，结果 review / 可信度 < 0.5 / 不收 AI 给的 owner 和 safe / 保留线索，请求里没有文件内容、有名字和子项名；整棵树 `--llm` → 问的次数 = AI 行数 = 服务器收到的请求数；设置没启用时 `--llm` 只提示不联网 |
| `b3_subtree_complete_and_export_tree_keeps_spine`（core） | 完整树 / 截断树的判定；导出只留 1 层时链上各层和它们的直接子项在、链外深处不在；摘要在完整树上算（http-v2 继承 pip-cache、owner = pip）；序列化平铺、能按 `Node` 读回 |
| `b3_outer_ancestors_from_real_dirs`（core） | 真实目录：从盘符根到上一层、从浅到深，最后一层带子目录和文件名；`C:\` 为空、`C:\Windows` 为 1 层 |
| `b3_msix_sandbox_redirect_and_standalone_cargo_target`（core） | 沙箱路径识别（不认 `LocalCache\Local` 本身、`Settings`、非包路径）；沙箱里的 pip 缓存按 `pip-cache`（精确 + 继承）并写明重定向；真实路径有更具体规则时照旧（`claude-desktop-vm`）；沙箱里的 cargo 编译目录及其子目录是 `rust-target`；认不出的退回 `claude-desktop-data` 并提醒；沙箱外的包数据不加提醒；沙箱外单独的 cargo 编译目录也认得；只有 `CACHEDIR.TAG` 不算 |

- `cargo check -p diskwhy`（Tauri 壳）通过。
- 真实环境：`explain C:\…\LocalCache\Local\pip --top 3` 在本机输出 `pip\cache` 141 MB“可清理（规则 pip-cache）”；`explain C:\Users\<用户>\AppData\Local\pip\cache\http-v2` 输出“pip-cache（继承上层）”。
- 查过的资料：learn.microsoft.com《Understanding how packaged desktop apps run on Windows》（AppData 新建文件重定向到每用户每包的位置）；Cargo Book “Build cache”（target 目录布局、`CARGO_TARGET_DIR`；页面里没提 `CACHEDIR.TAG` / `.rustc_info.json`）；rust-lang/cargo PR #8378（target 目录放 `CACHEDIR.TAG`）。

### 10.4 没做 / 没验证的

- **`--llm` 没有连真实模型跑过**，只用本机假服务器验证了“什么时候问、发了什么、结果怎么处理”（按要求默认不联网）。
- **没有在 GUI 里接这次的改动**：沙箱重定向和新的 `rust-target` 在 GUI 的列表 / 详情里会自动生效（同一个 `explain`），但没有重新构建 GUI、没截图；`cargo build --release` 只构建了 CLI（决定 31）。
- **全盘按规则库归档（`tier_summary`）没跟着改沙箱重定向**：沙箱里的 pip 缓存在那组 safe / review / keep 合计里仍算在 `claude-desktop-data`（review）下，列表和详情里的单项说明是对的。本机影响约 141 MB。
- `.rustc_info.json` 作为 cargo 编译目录标记，官方文档里没找到说明，是本机观察（决定 29）；如果将来 cargo 改名，这条启发式会失效（退回到包规则或未知，不会误判成 safe）。
- `DiskWhy-build` 这一层（只装着一个 cargo 编译目录）仍按 Claude 包规则说明（带提醒），没有专门处理。
- coverage.md 里两条“要查”的待办（Raspberry Pi Imager 缓存、`AppData\Local\OpenAI\Codex`）没有查证、没有写规则。
- 扫描时另一轮会话在同一块盘上编译，C 盘数据在变；两轮之间有几百 MB 的差异（卷已用 182.19 → 182.28 GB），不影响前 50 的结论。
- scan 的 JSON 现在按键名字母顺序输出（`serde_json::Value`），和以前的字段顺序不同；`--from` 读取不受影响，别的程序如果按顺序解析会受影响（目前没有）。

### 10.5 下一阶段从哪接

- 仍在 `D:\desk\claudex\13\diskwhy\.worktrees\phase-b` 做、提交到 `phase-b`，构建用 `CARGO_TARGET_DIR=D:/desk/claudex/13/diskwhy/target`。另一轮会话在主目录做阶段 D、会覆盖 `target/release/diskwhy-cli.exe`，本轮用的 CLI 另存了一份在 `target/tmp/diskwhy-cli-b3.exe`，扫描产物在 `target/tmp/`（`scan.json`、`c.dwsnap`、`explain50.json`、`report50.json`、`report200.json`、`cov200.md`；`r1/` 是第一轮）。
- 规则待办看 coverage.md 最后一节；`tier_summary` 接上沙箱重定向；GUI 构建截图。
- 用户醒来后要决定：阶段 D 那批改动怎么处理、`phase-b` 何时合回 master。
