# 阶段 B 验证记录：解释引擎

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
