# DiskWhy — 总体计划

> 工作名 DiskWhy：告诉你 C 盘每一块是什么、为什么在、删了会失去什么、怎么安全腾出来。
> 规划：Fable。执行：Opus 子代理，按阶段逐个交付，每阶段必须有可运行的验证证据。

## 0. 背景与定位（来自调研，见 ../mangodisk-eval/REPORT.md）

市面上可视化工具（WizTree/TreeSize/SpaceSniffer）只告诉你哪里大；清理工具（电脑管家/CCleaner/MangoDisk）只清它认识的固定条目。
最接近的 MangoDisk（开源 GPL，3k 星）实测只做到五成：**对陌生文件夹不能解释、少算 34 GB 且不提示、无回收站/隔离区、分析页删除防护弱、中文软件 Windows 端只有 17 条规则。**

DiskWhy 的定位就是填这些空白，四条产品原则：

1. **每一块都有归属**：已用空间 100% 有解释，扫不到的也要标出来是"无权限 / 系统保留 / 未归属"。
2. **每一块都有说明**：这是什么、属于哪个软件、为什么会有、删了会失去什么、能腾多少、怎么腾。
3. **说后果，不替用户拍板**：三档 = `safe`（缓存类，自动重建）/ `review`（删了有代价，需用户判断）/ `keep`（用户数据或系统关键，锁死不让删，但可建议迁移或用系统工具处理）。未知目录默认 `review`，绝不默认 `safe`。
4. **删除有护栏且可撤销**：默认进回收站或迁到其他盘的隔离区；保护路径硬编码；删前列出真正会删的全部内容；`keep` 档没有删除按钮。

## 1. 技术栈与工程约束

- Tauri 2 + Rust（后端/核心）+ Vue 3 + TypeScript + Vite（前端）。机器上已有 rustc 1.93、node 22、npm 10.9，无 .NET。
- Cargo workspace：
  - `crates/diskwhy-core`：扫描、规则、解释、动作。**纯库，带单元测试**，不依赖 Tauri。
  - `crates/diskwhy-cli`：命令行，JSON 输出，用于自动化验证。
  - `src-tauri`：Tauri 壳，只做 IPC 转发。
  - `src`：Vue 前端。
  - `rules/`：TOML 规则库，编译期 `include_str!`/`include_dir` 嵌入，运行时可加载用户自定义目录。
- 许可：MIT。**禁止复制 MangoDisk（GPL）任何代码或文案**；Winapp2 是 CC-BY-SA，不批量导入，只能作为"某路径属于某软件"的事实参考，说明文案全部自己写。
- 实现 Windows API 前先查官方文档（learn.microsoft.com / docs.rs `windows` crate / tauri.app v2 文档），不凭记忆。
- 每阶段结束：`cargo test` 全绿、`cargo build --release` 通过、CLI 能对真实 C 盘跑出 JSON、GUI 能启动并截图。截图存 `docs/screenshots/`。
- 界面中文优先，文案大白话。

## 2. 数据模型（core）

```
Node { path, name, kind: File|Dir, logical: u64, allocated: u64, mtime, attrs, children: Vec<Node>(仅 Dir), inaccessible: bool }
ScanResult { root, total_bytes, used_bytes, tree: Node, buckets: Attribution, duration_ms, files, dirs, elevated: bool }
Attribution { scanned, inaccessible, system_reserved(回收站/卷影/pagefile/hiberfil 等按名归类), cloud_placeholder(不计入可释放), unattributed = used - 其余 }
Explanation { tier: safe|review|keep, owner: Option<String>, category, what, why, lose, gain_bytes, how: Delete|BuiltinTool(name)|Migrate|Manual|None, source: Rule(id)|Heuristic(name)|Llm|Unknown, confidence }
```

- `allocated` 用 `GetCompressedFileSizeW` 取磁盘实际占用并按簇（`GetDiskFreeSpaceW`）向上取整；界面默认展示 allocated。
- 不跟随 reparse point（符号链接/junction/挂载点），只记其自身。
- 云占位文件（`FILE_ATTRIBUTE_OFFLINE` / `RECALL_ON_OPEN` / `RECALL_ON_DATA_ACCESS`）只读属性，绝不读内容，标记 `cloud_placeholder`。
- 硬链接（WinSxS 等）：阶段 A 先不去重，记 TODO；阶段 C 用 `nNumberOfLinks>1` 做去重估算。

## 3. 阶段划分

### 阶段 A：扫描引擎 + 归属统计 + 可用的壳（本次先做）
- core：多线程扫描（rayon + 手写 `FindFirstFileExW` 或 jwalk），进度回调，取消，上面的数据模型与 Attribution 计算。
- cli：`diskwhy scan C:\ --json out.json --top 30`，打印前 30 大目录 + 归属统计。
- tauri + vue：选盘 → 扫描（进度条、可取消）→ 面包屑下钻的大小列表（按 allocated 降序，显示占比）→ 顶部"归属条"（scanned / 无权限 / 系统保留 / 未归属 各占多少）。右侧详情面板先显示路径、大小、修改时间，为阶段 B 的说明预留位置。
- 检测是否管理员，不是就在界面提示"以管理员身份重新启动可看到系统目录"，并提供一键提权重启（`runas`）。
- 验证：cargo test（用临时目录构造树测试大小、占位、reparse、无权限处理）；CLI 对真实 C 盘跑一次，记录耗时；GUI 截图。

### 阶段 B：解释引擎（核心差异化）
- `rules/*.toml`，schema：`id, owner, category, tier, match = [路径模式，支持 %LOCALAPPDATA% 等环境变量和 * 通配], what, why, lose, how, refs, verified_on`。启动时校验：路径模式必须落在允许的根之下，`safe` 档不得指向用户文档类目录。
- 首批规则 ≥ 80 条，分组：Windows 系统项（WinSxS→只能用 DISM、Windows.old→磁盘清理、Installer→勿删、传递优化、休眠文件→powercfg、pagefile、Temp、Prefetch→勿删、缩略图缓存）；浏览器缓存（Chrome/Edge/Firefox 及 Chromium 系）；开发缓存（npm/pnpm/yarn/pip/uv/cargo/go/gradle/maven/nuget/conda pkgs/Docker/WSL vhdx/VS/JetBrains/VS Code/Cursor）；AI 模型（HF/Ollama/torch/ModelScope/insightface/paddle/LM Studio）；中国软件（微信/QQ/企业微信/飞书 LarkShell/钉钉/百度网盘/迅雷/网易云/QQ 音乐/酷狗/B 站/WPS/搜狗、微软拼音等输入法），微信 QQ 的聊天文件必须是 `keep + Migrate`。
- 启发式（规则未命中时）：目录内容签名（`Cache_Data`/`GPUCache`/`Code Cache` → Chromium 缓存；`site-packages` → Python 环境；`node_modules`；`target`+`Cargo.toml`；`*.safetensors|*.gguf|*.onnx|*.bin` 大文件 → 模型权重；`*.vhdx` → 虚拟磁盘；`*.log` 占多数 → 日志），文件名归属推断（注册表 `Uninstall` 键的 DisplayName/InstallLocation 与目录名匹配 → "属于已安装软件 X"），最后修改时间（超过 180 天未动的缓存提示"很久没用"）。
- LLM 兜底：默认关闭；设置里填 OpenAI 兼容端点（支持本机 Ollama），只发送目录名、父路径脱敏、子项名摘要、大小，绝不发文件内容；回复只作 `review` 档说明，不能提升为 `safe`。
- 界面：列表每行加档位色标和一句话说明；详情面板显示完整六项；未知项显示"我也不确定，这是我看到的线索：…"。
- 验证：规则 schema 测试；对本机真实 C 盘前 50 大目录跑解释，统计命中率并把结果写进 `docs/coverage.md`。

### 阶段 C：Treemap + 完整归属 + 打磨
- squarified treemap（Canvas 渲染），与列表联动，色块按档位着色，"无权限 / 系统保留 / 未归属"作为显式色块。
- 硬链接去重估算；管理员模式下扫 WinSxS/System32/Installer 并标 `keep`。
- 扫描结果快照保存/加载（JSON），便于复核。

### 阶段 D：带护栏的动作
- 删除仅对 `safe` 和 `review` 可用；`keep` 无按钮，只给"用系统工具处理"或"迁移"入口。
- 默认策略：回收站（`IFileOperation` + `FOF_ALLOWUNDO`，或 `trash` crate）；C 盘空间告急时提示"回收站在 C 盘，不会立刻腾空间"，并提供"迁到其他盘隔离区（保留 7 天自动清）"。
- 护栏：保护路径硬编码（Windows、Program Files、用户文档/桌面/图片、.ssh、OneDrive 根等）；删前列出实际会删的完整清单和总大小；跳过 30 分钟内修改过的文件；Restart Manager 检测占用进程并只报告不强杀；每次动作写日志到 `%LocalAppData%\DiskWhy\actions.log`。
- 系统项走内置工具：Windows.old / 更新缓存 → 调 `cleanmgr` 或 `IEmptyVolumeCache`；休眠 → `powercfg /h off`（需确认）；WinSxS → `DISM /StartComponentCleanup`。
- 迁移向导（最小版）：conda envs / Ollama / 微信文件 迁到 D 盘 + 改配置或建 junction。

## 4. 成功标准

1. 作者本机 C 盘：前 50 大目录里 ≥ 80% 有规则或启发式说明；已用空间 100% 有归属。
2. 全盘扫描（普通权限）< 90 秒。
3. 三个朋友按建议删了东西没出事。
