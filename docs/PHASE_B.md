阶段 B 是 DiskWhy 的“解释引擎”：给扫描出来的每个文件夹说清楚它是什么、属于谁、为什么在、删了会怎样、该怎么腾，只解释不删除。

# 阶段 B 验证记录：解释引擎

> 第 1 节是**现在的状态**（第 1 轮评审修复之后）；第 7 节是全部决定（1–42 是 B-1~B-4，43 起是第 1 轮评审修复）；第 8–11 节是 B-1~B-4 各自的记录；第 12 节是第 1 轮评审与修复。第一次做阶段 B 时的旧记录（原第 1–6 节）挪到了文末「历史」，里面的数字和说法已经过时。

日期：2026-09-23　机器：Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
工具链：rustc/cargo 1.93.0，node 22，Tauri 2，windows-sys 0.61.2，toml 0.9，ureq 3.4.2（native-tls）
以下输出均为本机实际运行的原文摘录。覆盖率细节见 [coverage.md](coverage.md)。

---

## 1. 当前状态（第 1 轮评审修复之后，2026-09-23 夜，分支 `phase-b`）

> 这一节是现在的样子。第一次做阶段 B 时的记录（179 条规则、47 个测试、“启发式永不 safe”、LLM 发 9 个字段等）已经过时，整段挪到了文末的「历史」一节；中间的决定、B-1~B-4 各小节保留原样，读的时候以本节和第 12 节为准。

### 1.1 规则库

`rules/*.toml` 共 **222 条**（编译期嵌入；运行时再加载 `%LOCALAPPDATA%\DiskWhy\rules\*.toml`，走同一套校验）：

| 文件 | 条数 | safe | review | keep | 其中 safe 只在本机观察过 |
|---|---:|---:|---:|---:|---:|
| windows-system.toml | 72 | 10 | 16 | 46 | 1 |
| dev-caches.toml | 42 | 16 | 13 | 13 | 2 |
| misc.toml | 40 | 8 | 22 | 10 | 3 |
| cn-apps.toml | 36 | 6 | 13 | 17 | 6 |
| browsers.toml | 19 | 8 | 2 | 9 | 0 |
| ai-models.toml | 13 | 0 | 12 | 1 | 0 |
| **合计** | **222** | **48** | **78** | **96** | **12** |

字段：必填 12 项（id / owner / category / tier / match / what / why / lose / how / how_detail / refs / verified_on）；可选 `scope`（tree / self）、`kind`（any / dir / file）、`when_child`、`owner_from_path`，第 1 轮评审后新增 `observed_only`（safe 没有官方出处时必须写 true，界面标“本机观察”）和 `verify`（目前只有 `chromium-disk-cache`：命中后按 Chromium 磁盘缓存的严格格式再核一遍）。规则里 84 个不重复的参考链接，82 个 curl 返回 200，2 个（Adobe、Epic 的帮助页）对脚本返回 403（反爬），浏览器能打开，Adobe 那页用 WebFetch 读过原文。

加载时的校验（有一条不过，那条规则就不加载；内置规则有一条不过，测试就失败）：字段齐全、枚举合法、id 唯一、模式以白名单 `%变量%` 开头、不含 `..`、keep 不许 delete、builtin_tool / migrate 必须写 how_detail、原始模式重复报错；**safe 档另外**：不许落在桌面 / 文档 / 图片 / 视频 / 音乐 / 下载 / OneDrive；不许含凭据、代码仓库、聊天数据、网页存储一类的段名（`.ssh`、`source`、`repos`、`WeChat Files`、`xwechat_files`、`Local Storage`、`IndexedDB`、`Login Data` 等 40 个）；变量后面至少一段写死的名字（只有 `%TEMP%` 本身例外）；带 `**` 的必须有 `when_child` 或最后一段是已知缓存目录名；必须有 refs 或 `observed_only = true`。**运行时**（取决于本机环境变量）：safe 模式展开后只有 1~2 段、或能匹配到用户目录 / Windows / Program Files / ProgramData / AppData / 个人文件夹本身或它们的上层、或 `%TEMP%` 的最后一段不叫 Temp / Tmp，这个模式跳过并给警告（`%LOCALAPPDATA%\Temp` 兜底照常生效）。

### 1.2 解释链和护栏

`explain_full`：规则（精确 → 商店沙箱按原位置 → 继承上层 tree 规则）→ 启发式 → （传了 oracle 且设置里开了开关）AI → 未认出。出口统一过 `enforce_guards`。第 1 轮评审后在规则命中和 `enforce_guards` 之间加了一道 `guard_rule_hit`：

1. `verify = "chromium-disk-cache"` 的通配规则（`**\Cache`、`**\Code Cache`、`**\GPUCache` 等），命中的那个文件夹必须通过和启发式同样严格的签名检查，否则 safe → review；有“其余小文件”合并时回磁盘把全部文件名读一遍再核（只读名字）。
2. safe 规则覆盖的地方先看内容：聊天软件的数据文件夹（路径里有 WeChat Files / xwechat_files / Tencent Files / WXWork）→ keep；有 `conda-meta` 的文件夹 → keep；虚拟磁盘文件 → keep；有 `.git` / `.svn` / `.hg` 的文件夹 → review；Office / WPS / PDF 等文档 → review。自己是、上层（到规则命中的那层为止）是、下面有这些东西，分别处理：自己或上层是 → 改用对应说明；下面有且 keep 级的占一半以上 → 整个 keep；否则 safe → review，“能腾多少”写成“其中约 X 可以清理，另有 N 处看起来是你的数据”。
3. review 规则只查虚拟磁盘，而且只对“继承来的、不是专门讲虚拟机的规则”查（商店版 WSL 的 `LocalState` 挂在 `store-app-package` 下面，规则给的办法是“重置”，重置会清空整个发行版）。
4. `enforce_guards`：AI / 未认出一律 review；启发式的 safe 只认 `chromium-cache`；任何来源的 safe 落在个人文件夹下 → review。个人文件夹除了 `%USERPROFILE%\Documents` 这类默认位置，还用 `SHGetKnownFolderPath` 读真实位置（“文档”被移到 D 盘也保护）。

`tier_summary`（全盘各档合计）走同一套护栏：陷阱字节改记到 review / keep（`guarded_bytes`、`guard_items`），verify 核不过的记到 review，商店沙箱里的东西按原位置找规则，认不出记“规则没覆盖”，不再算到包规则上；另外单独给出 safe 里“要用工具清、不能直接删文件夹”的字节（`safe_tool_bytes`，给阶段 D 的批量删除用）。

### 1.3 启发式（规则没命中时，按顺序）

chat-data（keep）→ chromium-cache（**唯一能 safe 的**，签名严格）/ chromium-cache-mixed → conda-pkgs / conda-env（keep）/ python-venv / python-env → site-packages 里面 → app-bundled-modules（keep）/ node-modules / node-modules-orphan → rust-target（含 `CARGO_TARGET_DIR` 单独放的目录、以及几乎只装着一个 cargo 编译目录的外层文件夹）→ dotnet-build → model-weights → virtual-disk（keep）→ logs-dumps → downloaded-installer → temp-files → media-files → 注册表归属 → **空的**（0 字节，显示“不占空间”）→ 未认出。链接 / 挂载点（重解析点）在最前面单独处理：没有专门规则时一律“链接，不占空间，不用管”。

未认出的说明只说一次“还没认出来，先别动”，“删了会失去什么”写“先不动它不会有任何损失”，怎么腾是“先不用管”；线索第一条替用户总结一句（比如“很可能是「OpenAI」/「Codex」相关的程序文件（.exe / .dll 占 95%）”），读不了的写“普通权限读不了”。

### 1.4 AI 兜底（默认关闭）

请求体只有 4 个字段：名字、最近两层父路径（`X:\Users\<名字>` → `~`）、最多 20 项子项摘要（**子文件夹发名字，文件只发“.扩展名 文件 × 个数”，不发文件名**）、总大小；名字里的用户名（≥ 2 个字，中文名也算）、邮箱、`wxid_…`、5 位以上数字都打码。只有“未认出”的项能问（界面上只对这种项显示按钮，core 的 `llm::ask` 也会再挡一次，Tauri 的 `ask_ai` 走的就是它）。远程地址必须 https，只有 localhost / 127.x / [::1] 可以 http。回复只收 what / why / lose，档位固定 review、可信度 0.3、能腾出 0、归属只看注册表。

### 1.5 界面

欢迎页和扫描中常驻“四种颜色是什么意思”卡；口径说明折叠；进度过 90% 显示“正在收尾”和走动条纹。结果页：归属条改成中性的石板色 + 纹理（彩色只留给档位）；汇总卡叫“这些空间分别是什么”，副标题写规则命中的最大文件夹名，点卡片让列表只看这一档；图例每档常驻一句话 + “这一层 N 项”，小于 1 MB 的未认出项合并成“另有 N 个没认出来的小文件夹”，说明来源只分“规则库 / 推测”（有 AI 结果时多一个 AI）。每档只用一个名字：**可清理 / 需判断 / 保留 / 未认出**（另有灰色虚线的“不占空间”）。保留改成蓝灰色 + 锁，方块图里保留块加淡斜纹、“读不了”改成白底斜纹、可选“只高亮可清理”。详情里“用这个工具”是普通文字不是按钮；来源后面标“本机观察”。仍然没有删除按钮。

### 1.6 测试

`cargo test -p diskwhy-core -p diskwhy-cli`：**87 个通过，0 失败，2 个忽略（B-4 时 79 个；忽略的两个要设环境变量才跑）**。第 1 轮评审后新增 `crates/diskwhy-core/tests/r1_tests.rs`（6 个，其中 `safety_trap_corpus` 把安全评审探针 A、C 两节的陷阱原样搬进来，TEMP 取默认、`D:\`、`C:\Users\bob`、`C:\`、`C:\Users\bob\AppData\Local` 五种）和 CLI 单元测试 2 个；前端 `vue-tsc` 通过、`vitest` 7 个通过。

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
| 34 | （B-4）任务要“Tauri IPC 新增 explain 命令”，但列表每行的解释已经随 `list_dir` 一次带回 | 新增 `explain(path, kind?)`（直接转发 core 的 `explain`，不联网、不问 AI），和 `node_info` 共用同一个“找节点 + 建 Target”的函数；**列表仍由 `list_dir` 一次带回全部解释**，`explain` 给详情面板在节点没带解释时补取 | 一个文件夹几百行，每行再走一次 IPC 会让列表逐行闪出来；两条路径最终调同一个 core 函数，结果一致（用 WebView2 远程调试在真实窗口里调用核对过，见 11.2） |
| 35 | 档位颜色 | safe 绿、review 琥珀、**keep 改成石板灰 + 锁图标**（原来是红色）、未知改成淡紫 + 问号 + 虚线框；方块图同一套颜色 | 任务写“keep 灰锁”；红色容易被读成“危险、快删”，而 keep 的意思恰恰是“别动”。未知不能也用灰色（会和 keep 混），用和三档都不像的淡紫，并加问号 |
| 36 | “解释来源”图例放哪、怎么标 | 在概览和列表之间加一条常驻的图例条（收起概览时也在）：左边四种颜色、右边三种来源（规则 / 推测 / 未知；有 AI 结果时多一个 AI），每种后面是**当前文件夹里的项数**；列表每行在档位标签后面加同款来源小标签（规则实线框、推测虚线框） | 图例光告诉颜色不够，数字能让人一眼看出“这个文件夹有多少是真查到的、多少是猜的” |
| 37 | “BuiltinTool 显示工具名”：规则里只有 `how_detail` 一段话，没有单独的工具名字段 | 在 core 加 `Explanation.tool`（PLAN 第 2 节的 `BuiltinTool(name)`），由新文件 `explain/tools.rs` 从 `how_detail` 里认：一张约 30 项的对照表（DISM、powercfg、磁盘清理、“设置 > 系统 > 存储”、conda clean、pnpm store prune……）+ 浏览器“清除浏览数据”带上浏览器名 + 软件菜单“→ 设置”；**在文字里最先出现的那个**算主工具（规则文案都是先写推荐办法再写备选）。在 `enforce_guards` 出口统一填，JSON 里没有工具时不出现这个键 | 不给 203 条规则加新字段：59 条 builtin_tool 规则逐条手填容易漏；测试 `b4_every_builtin_tool_rule_names_its_tool` 要求 59 条全部认得出（当前全部认得出，逐条列表见 `_ops/S22` 的测试输出）。缺点：新写规则时如果用了表里没有的工具，只会退到“某软件自己的设置”或不显示，测试会拦住“完全认不出”的情况 |
| 38 | 界面主题 | 只保留浅色：删掉 `styles.css` 里 3 段 `prefers-color-scheme: dark`，`color-scheme: light`，底色改成偏暖的纸白 `#f5f5f0`；`tauri.conf.json` 窗口加 `"theme": "Light"`（查了本机 tauri-utils 2.9.3 源码里 `WindowConfig.theme` 的说明：“The initial window theme. Defaults to the system theme. Only implemented on Windows and macOS 10.14+.”，v2.tauri.app 配置页只列出了 Theme 取值 Light/Dark） | 任务要求“浅色底、深色字，不要黑底”；这台机器系统是深色主题，阶段 A 的截图就跟着变成了黑底 |
| 39 | 详情面板窄（1280×800 逻辑像素时只有 340px），六项左右两栏排时每个字段都折成三四行 | 改成“小标题在上、内容在下”，内容用满整行 | 同样的高度能多放一倍的字；“怎么腾”里的工具名和迁移建议不用滚很远 |
| 40 | 截图文件名 | 新截图叫 `phaseB-b4-1…6-*.png`，**不覆盖**第一次做阶段 B 时的 `phaseB-1…7` | 旧图记录的是当时的界面（红色 keep、深色系统下的样子），留着方便对比；文件名都以 `phaseB-` 开头，符合任务要求 |
| 41 | 截图里的用户名 | 截图脚本用 UI Automation 找出**名字里含用户名（区分大小写）**的控件，把它们在截图上的区域打马赛克（面包屑里的用户名按钮、详情里的完整路径）；列表切到“只看列表”，不截方块图（方块图画在 canvas 上，文字读不出来也就遮不了） | 第一次用不区分大小写匹配，把 `Kingsoft`（WPS）整行也遮了，改成区分大小写。含用户名的试拍图已删除 |
| 42 | worktree 里没有 `node_modules`，`npm run tauri build` 跑不了 | 在 worktree 里建了一个指向主目录 `node_modules` 的目录联接（junction），没有重新下载依赖 | 两边 `package.json` / `package-lock.json` 完全一致（`git diff master phase-b` 为空）；`node_modules` 在 `.gitignore` 里，不会进提交。以后删 worktree 时要先 `rmdir` 这个联接，别递归删 |
| 43 | （第 1 轮修复）safe 规则下面哪些东西算“用户数据陷阱” | 聊天数据文件夹（路径段）、有 `conda-meta` 的文件夹、虚拟磁盘文件 → keep；有 `.git/.svn/.hg` 的文件夹、Office / WPS / PDF 文档 → review。**文档只按单个文件算**，不因为某个文件夹里有一份 PDF 就把整个文件夹当成文档；下面 keep 级陷阱占一半以上字节才把整个文件夹当 keep，否则 safe → review 并在“能腾多少”里分开写 | 评审建议里的 `package.json`、`Cargo.toml`、`*.sln` 没算：pip / npm 在 Temp 里的构建目录（`pip-req-build-…`、解包的源码包）常带这些文件，全算进去 Temp 会满屏“需判断”；`.git` 才是“可能有没提交改动”的直接证据。按文件算文档，是为了不让一份 PDF 把整个 Temp 的 5 GB 都拖成 review（汇总里只把那 30 MB 文档挪出来） |
| 44 | review 规则下要不要也查陷阱 | 只查虚拟磁盘文件，而且只对**继承来的**、`category` 不是 `vm` 的规则 | 评审的第 8 条（商店版 WSL 的 LocalState 继承 `store-app-package`，办法是“重置”）；专门为某个位置写的 review 规则（比如 Claude 的虚拟机镜像 `claude-desktop-vm`）已经知道里面是 vhdx，不该被改成 keep |
| 45 | safe 模式展开后位置不对怎么办 | **跳过这个模式并警告**，不是把整条规则降级；判断条件：展开后 ≤ 2 段、能匹配到用户目录 / Windows / Program Files / ProgramData / LOCALAPPDATA / APPDATA / PUBLIC / OneDrive / 个人文件夹本身或它们的上层、`%TEMP%` 的最后一段不叫 Temp / Tmp | 同一条规则里其他模式（`%LOCALAPPDATA%\Temp`）照常起作用；警告出现在设置按钮的悬停提示和 CLI 输出里 |
| 46 | 评审建议“safe 模式变量后至少 2 个字面段” | 改成**至少 1 个**写死的段（只有 `%TEMP%` 本身例外；只有通配段时必须配 when_child），另加段名黑名单和 `**` 的限制 | 要求 2 段会把十几条正常规则（`%LOCALAPPDATA%\npm-cache`、`%SystemRoot%\Temp`、`%LOCALAPPDATA%\D3DSCache`、`%ProgramData%\Blizzard Entertainment` 等）全部打掉；评审真正想挡的 `%LOCALAPPDATA%`、`%SystemDrive%`、`%USERPROFILE%\.ssh`、`%USERPROFILE%\source\repos` 用“至少 1 段 + 黑名单”已经挡住，测试里逐个有用例 |
| 47 | 通配 Chromium 缓存规则怎么和启发式对齐 | 规则加 `verify = "chromium-disk-cache"`，命中后用启发式同一个函数核；继承命中时核的是规则命中的那一层（比如 `Cache`），不是当前子目录；有“其余小文件”时回磁盘读全部文件名 | 决定 9 的标准（名字 + 每个文件名都是标准格式）同样适用于规则；实机有 1.57 GB 名字叫 Cache 但混着别的文件的文件夹被改记到“需判断” |
| 48 | CacheStorage / ScriptCache | 单独一条 review 规则 `chromium-sw-storage`，只用两条 `**` 通配（覆盖所有浏览器和 Electron 应用），删掉原来的 safe 通配规则 | 评审两人都指出 Cache API 是网页应用自己管理的存储；Chrome 的“清除浏览数据”到底清不清它，这轮也没核实（评审的抓取超时，我没再查），但改成 review 不需要这个结论 |
| 49 | 26 条没有出处的 safe 规则 | 新字段 `observed_only`；能找到官方出处的补出处（13 条），找不到的如实标本机观察（12 条），微信 radium 改成了 review（1 条），名单写进测试 | 评审给了两个选项（补出处 / 标观察）；找不到官方出处的国内软件、显卡驱动着色器缓存、Steam、VS Code 缓存不硬凑链接 |
| 50 | 实机 Temp 被标成“需判断” | 接受 | Temp 里真有 19 个 PDF（包括看起来是用户自己的“离线合订本”），这正是评审第 1 条要防的；汇总卡里 Temp 的 5.13 GB 仍算“可清理”（只挪出 30 MB 文档），列表里 Temp 这一行是“需判断”并写明“其中约 5.14 GB 可以清理” |
| 51 | AI 请求再收紧到什么程度 | 文件不发名字只发“扩展名 × 个数”；子文件夹名照发（脱敏后）；父路径只留最近两层；用户名 ≥ 2 个字就替换（1 个字的不换，否则会把名字里所有同样的字母都换掉）；邮箱、`wxid_…`、5 位以上数字打码；“问 AI”只对未认出开放，挡在 core 的 `llm::ask` 里（Tauri 和以后别的前端都走它） | 评审第 9 条逐项照做；子文件夹名是猜“这是什么软件”最有用的信息，不发的话 AI 基本没法猜 |
| 52 | `SHGetKnownFolderPath` 放在哪 | 只在 `ExplainCtx::load_default`（生产）里调，`ExplainCtx::new` 保持纯函数；新增 `add_protected` 给测试和以后用 | 测试用假环境变量造上下文，不应该读到本机真实的“文档”位置 |
| 53 | 空的、链接怎么显示 | core 里是两个启发式（`empty` review、`reparse-link` keep，how 都是“先不用管”，能腾 0）；界面把它们单独显示成灰色虚线的“不占空间”，不算进四种颜色 | 评审要“0 B 不进未知”“链接统一显示不用管”；新增一个档位会动阶段 D 的删除逻辑，所以只在界面上区分 |
| 54 | 叶子覆盖率怎么算 | 每行的独占字节 = 自己 − 表里最近一层嵌在它里面的行；只有非容器规则、继承、启发式算“覆盖”；前 50 的结果 58.9% 如实写 | 评审第 3 条原话；没有为了让它过 80% 去改口径或补容器说明 |
| 55 | `explain --json` 里的用户名 | 新参数 `--redact`，默认**不**脱敏 | `--json` 是给程序用的，路径要能直接拿去用；要分享时加 `--redact`。`--md` 本来就给人看，一直脱敏 |
| 56 | 评审要求“先合并 phase-b 到 master” | **不合并**，只用 `git merge-tree --write-tree master phase-b` 试合，把冲突列表写进 12.1 | 任务硬约束“main 分支一个字不改”（本仓库的主分支是 master，决定 2）。`git merge-tree` 只在对象库里算一棵树，不动任何分支、索引和工作区 |
| 57 | 阶段 D 的“批量删 safe”怎么避开只能用工具清的 | 不加新的档位；汇总里单独给 `safe_tool_bytes`，12.4 写明阶段 D 批量删只选 `tier = safe && how 含 delete` | 阶段 D 代码在 master 上，这条分支不改删除逻辑 |
| 58 | 驱动解压目录（`C:\AMD` 等）、`C:\Temp`、`C:\tmp` | 都是 review；`C:\tmp` 按“可能是你自己放的”写 | 查了 AMD、NVIDIA 的支持页，没找到“这个文件夹能删”的官方说法（搜到的都是论坛和第三方文章）；本机 `C:\tmp` 里确实有用户自己的东西 |
| 59 | 截图怎么拍、怎么打码 | 用 WebView2 远程调试端口（CDP）点界面按钮、`Page.captureScreenshot` 截页面（视口 1280×800 × 1.25，和冷启动评审一样）；截图前把页面文字里的用户名换成 `<用户>`；canvas 上画的用户名和扫描中实时刷新的“当前路径”用图片打码；旧截图挪到 `old/` 并打码 | 上一轮的 UI Automation + PrintWindow 脚本在临时目录里已经没了；CDP 能直接读 DOM 确认页面里还有没有用户名（脚本每张都检查并打印） |

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

- 真实环境：`diskwhy-cli explain C:\Users\<用户>\AppData\Local\pip --top 5` 用本机真实环境变量加载规则，**没有任何规则警告**，`pip\cache` 174 MB 由 `pip-cache` 说明为“可清理”（204 条 = 内置 200 + 本机 `%LOCALAPPDATA%\DiskWhy\rules\phaseD-demo.toml` 里另一轮会话留下的 4 条演示规则，没动它）。
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
| `C:\Users\<用户>\AppData\Local` | 31 s | 73,148 | 1.9 s | 规则精确 970 / 继承 63,215；启发式判 safe 的只有 1 处（Playwright 浏览器配置里的 `Shared Dictionary\cache`，264 KB）；`app-bundled-modules` 26 处（Typeless、Notion 自带依赖，keep）；未知 3,797 个文件夹 21.1 GB |
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

## 11. B-4：GUI 接入（S22，分支 `phase-b`）

### 11.1 做了什么

先对照任务逐条核。第一次做阶段 B 时 GUI 已经有：列表色标 + 一句话、详情六项、未知项线索 + “问 AI”、顶部各档合计、设置页（AI 开关默认关）。B-4 补的是：

| 任务要求 | 状态 | 位置 |
|---|---|---|
| Tauri IPC 新增 `explain` 命令（转发 core） | **本轮加** `explain(path, kind?)`；和 `node_info` 共用 `with_target`（决定 34） | `src-tauri/src/lib.rs`、`src/api.ts` 的 `explainPath` |
| 列表每行档位色标：safe 绿 / review 琥珀 / keep 灰锁 + 一句话 | **本轮改**：keep 从红色改为灰色 + 锁，未知改为淡紫 + 问号；新组件 `TierChip`（对勾 / 三角 / 锁 / 问号四个小图标）；每行多一个来源小标签 | `src/components/TierChip.vue`、`DirList.vue`、`styles.css`、`TreemapView.vue` 的配色 |
| 详情六项：这是什么 / 属于哪个软件 / 为什么会有 / 删了会失去什么 / 能腾多少 / 怎么腾 | 标签改成任务原文；排版改为上下排（决定 39） | `DetailPanel.vue` |
| 怎么腾：BuiltinTool 显示工具名，Migrate 显示迁移建议，无删除按钮 | **本轮加** `Explanation.tool`（决定 37），面板里“用这个工具：DISM 命令（管理员终端）”；迁移类的做法小标题是“迁移建议”；面板底部写明“只解释、不删除：没有删除按钮” | `crates/diskwhy-core/src/explain/tools.rs`、`mod.rs`、`DetailPanel.vue` |
| 未知项显示“我也不确定，这是我看到的线索：…” | 已有；**本轮把线索框挪到六项之前**，未知项一选中先看到线索 | `DetailPanel.vue` |
| 顶部“解释来源”小图例（规则 / 启发式 / 未知） | **本轮加**，带当前文件夹的计数（决定 36） | `src/components/ExplainLegend.vue`、`App.vue` |
| 设置页 LLM 兜底开关（默认关）与端点填写 | 已有；开关文字改成“启用 AI 推测（默认关闭；关着时 DiskWhy 不会联网）” | `SettingsDialog.vue` |
| 中文、大白话、清新（浅色底、深色字，不要黑底） | **本轮改**：去掉深色模式、纸白底、窗口主题固定 Light（决定 38） | `styles.css`、`src-tauri/tauri.conf.json` |

### 11.2 怎么验证的

- `cargo test -p diskwhy-core -p diskwhy-cli`：**79 个通过，0 失败，2 个忽略**（B-3 时 76 个；新增 `explain::tools` 单元测试 1 个、`tests/b4_tests.rs` 2 个：59 条 builtin_tool 规则全部认得出工具名且不超过 40 字、百度网盘先写的是软件自己的设置所以不算“已安装的应用”；WinSxS 的解释带 `tool = "DISM 命令（管理员终端）"` 且 JSON 里有这个键；把 how 改成只有迁移后过护栏，`tool` 消失、JSON 里也没有这个键；未知项没有工具名、有线索）。59 条规则各自认出的工具名存在 `D:\desk\claudex\_ops\S22\B-4-tools.txt`，逐条看过。
- 前端：`vue-tsc --noEmit` 通过；`vitest run` 7 个通过（阶段 C 的方块图布局测试，没有新增前端测试）。
- 构建：`npm run tauri build -- --no-bundle`（worktree 里跑，`CARGO_TARGET_DIR` 指回主目录 `target/`）通过，产物 `D:\desk\claudex\13\diskwhy\target\release\diskwhy.exe`（第一次 4 分 06 秒；改了详情排版后重建 2 分 53 秒）。这次的 exe 另存了一份 `target\tmp\diskwhy-b4.exe`，防止主目录的另一轮会话覆盖。
- **`explain` 命令在真实窗口里调过**：用 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动发布版、GUI 里扫完 C 盘后，通过 Chrome DevTools 协议在页面里执行 `window.__TAURI_INTERNALS__.invoke("explain", …)`，走的就是前端 `explainPath` 那条 IPC。结果摘要如下（原始 JSON 存在 `D:\desk\claudex\_ops\S22\B-4-ipc-explain.txt`，用户名已换成 `~`）：

```
C:\Windows\WinSxS           => keep  rule winsxs      tool "DISM 命令（管理员终端）"  how [builtin_tool]
~\AppData\Local\Temp        => safe  rule user-temp   tool "Windows 设置里的“存储”"   how [delete, builtin_tool]
~\AppData\Local\wsl         => keep  rule wsl-root    tool null                       how [migrate]
~\AppData\Local\OpenAI      => review unknown         tool null  线索 6 条  what "我也不确定这是什么"
C:\Windows\System32 (Rest)  => keep  rule system32（其余小文件跟随所在文件夹）
C:\swapfile.sys (File / 省略 kind) => keep rule swapfile
C:\does-not-exist           => 错误“在扫描结果里找不到：C:\does-not-exist”（没有崩）
C:\Windows (Rest)           => 错误“在扫描结果里找不到：C:\Windows”——C:\Windows 顶层只有 26 个文件，不到每个文件夹保留的 50 个，没有“其余小文件”节点，报错是对的
```

- GUI 扫描：普通权限，GUI 里扫了三次 C 盘（第一次 68 秒、第二次 63 秒，第三次是为调试通道重开后扫的、没计时），约 109.5 万个文件、224,506 个文件夹。

**截图**（`docs/screenshots/`，发布版，普通权限，窗口 1600×1000 物理像素 = 1280×800 逻辑像素；方法同阶段 A/C：UI Automation 的 `InvokePattern` 点按钮和行、`ScrollPattern` 滚动详情面板、`PrintWindow(PW_RENDERFULLCONTENT)` 抓窗口；用户名区域打码，见决定 41）：

| 文件 | 内容 |
|---|---|
| `phaseB-b4-1-root-list-legend.png` | 扫完 C 盘：概览下面是新的图例条（颜色：可清理 0 / 需判断 3 / 保留 8 / 未知 9；解释来源：规则 9 / 推测 2 / 未知 9）；列表每行灰色锁“保留” + “规则”标签 + 一句话；`tmp`、`AMD`、`Temp` 等是淡紫问号“未知”；右侧 C:\ 本身的六项 |
| `phaseB-b4-2-safe-temp-tool.png` | `AppData\Local`（概览已收起，图例条还在）：选中 `Temp`，绿色“可清理”；怎么腾写“可以直接删除，或用系统或软件自带的工具清理”，下面蓝底 **“用这个工具：Windows 设置里的“存储””** 和具体做法 |
| `phaseB-b4-3-keep-lock-winsxs-dism.png` | `C:\Windows`：111 项全部来自规则；选中 `WinSxS`，灰锁“保留”，能腾多少“不能手动删；用系统自带工具清理能腾出一部分（最多 3.65 GB）”，**工具 DISM 命令（管理员终端）** + 两条 DISM 命令 |
| `phaseB-b4-4-keep-migrate-wsl.png` | `AppData\Local\wsl\{…}`：WSL 发行版的 ext4.vhdx，灰锁“建议保留，别删”，六项完整可见；怎么腾“迁到别的盘”，**“迁移建议：”** `wsl --manage … --move` / export-import |
| `phaseB-b4-5-unknown-clues.png` | `AppData\Local\OpenAI`：淡紫问号“还没认出来，先别删”，六项之前先是 **“我也不确定，这是我看到的线索：”** 6 条（最大子项 Codex\、扩展名分布、文件数、修改时间、在 AppData 下、注册表里对不上） |
| `phaseB-b4-6-settings-llm-off.png` | 设置页：“启用 AI 推测（默认关闭；关着时 DiskWhy 不会联网）”未勾选，接口地址 / 模型名 / API Key 都是空的，下面写着会发送的四样东西和绝不发送文件内容 |

截图全程没有保存过设置，`%LOCALAPPDATA%\DiskWhy\settings.json` 不存在（AI 保持默认关闭）。截图时把界面切到了“只看列表”，结束前已通过同一个调试通道把这个偏好改回默认的“方块图 + 列表”。

### 11.3 没做 / 没验证的

- **没截方块图**：方块图的文字画在 canvas 上，UI Automation 读不到，用户名没法按控件打码（决定 41）。方块图用的是同一套新颜色（keep 灰、未知淡紫），只在一张没保存的试拍图里看过，没放进文档。
- **“问 AI”这次没在 GUI 里重跑**：设置保持默认关闭，没连真实模型（第 4 节的 phaseB-5/6/7 是上午用旧界面拍的）。AI 结果区块的标签改成了和六项一致（加了“能腾多少”），但没有新截图。
- **列表行和图例的“来源”没有 AI**：AI 结果只显示在详情面板里、不回写列表，所以图例里的 AI 计数实际上总是 0（代码里有，只在出现时显示）。
- `explain` 命令在界面里只作兜底（节点没带解释时才调用）；正常操作下列表、方块图点选都已带解释，所以这条兜底路径在 GUI 里**没有被自然触发过**，只用调试通道直接调过命令本身。
- 工具名靠对照表从文案里认，是启发式的（决定 37 写了缺点）；新写的 builtin_tool 规则要跑一遍 `b4_tests`。
- 窗口 `theme: "Light"` 只在这台（系统深色主题的）Windows 11 上看过效果：标题栏和页面都是浅色。
- 观察到但没处理：`AppData\Local` 这一层 120 项里有 75 项是“未知”（多是小文件夹），前 50 大目录覆盖率 100% 不代表每一层都认得全；属于规则库的下一批工作，不是 GUI 问题。
- 前端没有新增单元测试（新组件都是展示逻辑）；验证靠 `vue-tsc`、真实窗口截图和 IPC 调用。

### 11.4 下一阶段从哪接

- 仍在 `D:\desk\claudex\13\diskwhy\.worktrees\phase-b` 做、提交到 `phase-b`；构建用 `CARGO_TARGET_DIR=D:/desk/claudex/13/diskwhy/target`。worktree 的 `node_modules` 是指向主目录的联接（决定 42）。
- 截图脚本（UI Automation + PrintWindow + 按控件名打码）在本次会话的临时目录里，思路见决定 41，重写很快；若要长期用，可以放进仓库的 `scripts/`。
- 合并：master 现在是另一轮会话提交的阶段 D（eb911fb），也改了 `DetailPanel.vue`、`App.vue`、`styles.css`、`lib.rs`；`git merge phase-b` 这几个文件**一定会冲突**，要手动合：保留阶段 D 的清理按钮 / 抽屉，同时保留 B-4 的六项标签、工具名、图例、浅色主题和 `explain` 命令。
- 规则库：`AppData\Local` 里大量小的未知项、coverage.md 末尾的待办。

## 12. 评审与修复（第 1 轮：产品 39 分、安全 31 分、冷启动 31 分）

三份评审原文在 `D:\desk\claudex\_ops\S22\评审-r1-{product,safety,cold-start}.md`。先严重后一般。提交：a4383f6（安全核心）、53c27b7（AI 脱敏 / 已知文件夹 / 未认出文案 / CLI）、8d944a9（界面）、以及本节所在的提交（截图、文档、覆盖率）。

### 12.1 逐条处理

**安全评审**

| # | 级别 | 问题 | 怎么改的 | 怎么验证 |
|---|---|---|---|---|
| 1 | 严重 | 继承的 safe 规则盖过保护性启发式，Temp / INetCache 里的聊天、git、conda、vhdx、文档都算 safe，还进了“可放心清理”总数 | `guard_rule_hit` + `heuristics::trap_here / traps_below`（1.2 节）；`tier_summary` 同步改记（决定 43、44） | `r1_tests::safety_trap_corpus`：13 个陷阱本身、它们下面每一层都不是 safe；`tier_summary` 的 safe 字节正好等于真正的临时文件和缓存（22,001,000 = 20 MB .tmp + 1 MB pip 解包 + 1.001 MB 真 Chromium 缓存）。**变异检查**：把 `guard_rule_hit` 拿掉，这个测试立刻失败（“Temp\x\WeChat Files 被标成了 safe（规则 user-temp（继承））”），恢复后通过 |
| 2 | 严重 | safe 模式展开后不查位置，TEMP=`D:\` 时整个 D 盘 safe | `rules::safe_location_problem`：展开后 ≤ 2 段、能匹配到用户目录 / Windows / Program Files / ProgramData / AppData / 个人文件夹本身或上层、`%TEMP%` 最后一段不叫 Temp/Tmp → 跳过这个模式并警告（决定 45） | 同一个测试里 TEMP 取 `D:\`、`C:\Users\bob`、`C:\`、`C:\Users\bob\AppData\Local`：都有“user-temp … %TEMP% … 跳过”的警告，D 盘上的微信 / 虚拟机 / conda / 普通文件夹都不是 safe，D 盘 `tier_summary` 的 safe = 0，`%LOCALAPPDATA%\Temp` 兜底仍然生效，C 盘陷阱也都不是 safe |
| 3 | 一般 | Temp 里从压缩包 / Outlook 打开的文档，lose 写“不丢数据” | 新规则 `temp-opened-from-archive`（`Rar$EX*`、`Rar$DI*`、`7zO*`、`Temp?_*`）、`outlook-attachment-temp`（`INetCache\Content.Outlook`），都是 review，lose 写“改过的这一份只在这里”；另外文档类文件在 safe 规则下一律 review（第 1 条）；user-temp、inetcache 的 lose 加了例外说明 | 测试核对两条规则的档位和文案；实机 C 盘 `Temp` 因为里面有 PDF 被标成“需判断”（决定 50） |
| 4 | 一般 | 通配 Chromium 缓存规则比启发式宽松 | 三条通配规则加 `verify = "chromium-disk-cache"`（决定 47） | `Cache` 里放 `data_0` + `my_notes.txt` → review 并写明“混着别的东西”；真的 `Cache\Cache_Data\{index,data_0,f_…}` 仍 safe；实机全盘有 1.57 GB 这类“名字像、内容不全是”的缓存被改记到 review |
| 5 | 一般 | Service Worker\CacheStorage 标 safe | 从 chrome / edge / brave / vivaldi 规则里拿掉，删掉 `chromium-generic-sw-cache`，新规则 `chromium-sw-storage`（review，CacheStorage + ScriptCache，how_detail 只写“关掉程序后删”，lose 写离线内容会丢）（决定 48） | 规则测试；实机全盘这一类 3.47 GB 从“可清理”移到了“需判断” |
| 6 | 一般 | JetBrains `caches` 可能连带 Local History | `caches` 拆成 review 规则 `jetbrains-caches-dir`，how 优先 IDE 里的 Invalidate Caches 并写明别勾那个选项；`index / log / tmp` 仍 safe。依据：官方页原文“Clear file system cache and Local History”是同一个选项（WebFetch 读过）；“手动删 caches 会丢 Local History”仍是推断 | 规则测试 |
| 7 | 一般 | 校验器只挡 6 个个人文件夹 | safe 段名黑名单、变量后至少一段写死的名字、`**` 必须有 when_child 或缓存名（决定 46） | `validation_rejects_bad_rules` 新增 9 个用例（.ssh、source\repos、只有变量、只有通配、`**\Data`、聊天数据、Local Storage、没有出处、verify 写错），全部被拦 |
| 8 | 一般 | 商店版 WSL 的 LocalState 是 review，办法是“重置” | 新 keep 规则 `store-wsl-distro-state`（`Packages\*\LocalState`，when_child = ext4.vhdx），how_detail 分步迁移并写“千万别用重置”；另外 review 继承规则下的虚拟磁盘也会升到 keep | 测试：LocalState → keep，how_detail 含“别用“重置”” |
| 9 | 一般 | AI 脱敏只处理用户名；“问 AI”对启发式和 keep 也开放 | 1.4 节（决定 51） | `r1_tests::llm_redaction_and_gates`：评审探针 E 节的原样例子（张三的简历.docx、手机号、wxid、邮箱.ost、id_rsa、两字中文名、非 Users 路径）请求体里一个都找不到；规则认得的、启发式认得的都不能问 |
| 10 | 一般 | 文档说查了文件夹重定向，代码里没有 | `win::known_personal_folders`（`SHGetKnownFolderPath` × 6 个 FOLDERID，按 learn.microsoft.com：flags 0、token NULL、不管成败都 `CoTaskMemFree`、返回值不带结尾反斜杠），`load_default` 里加进受保护目录（决定 52） | `r1_tests::known_folders_are_protected`：真实系统读到 ≥ 2 个、都是绝对路径；假“文档”在 `D:\我的文档` 时那里的严格 Chromium 缓存也只能 review |
| 11 | 一般 | 护栏测试没覆盖“内置规则 + 继承” | 就是 `safety_trap_corpus` | 见第 1 条 |
| 12 | 建议 | 26 条 safe 没有出处 | 新字段 `observed_only`；26 条里 13 条补了官方出处（Edge 帮助、Chromium 磁盘缓存设计文档、Crashpad 设计文档、磁盘清理说明、内存转储说明、WER LocalDumps、商店应用临时数据、Electron 安装文档、node-gyp、Adobe 媒体缓存、rpi-imager #209），12 条如实标 `observed_only = true`，微信 radium 改成 review | 校验器要求 refs 或 observed_only；`r1_tests::safe_rules_have_provenance` 把 12 条本机观察的名单写死，以后新增要显式登记 |
| 13 | 建议 | 只写 builtin_tool 的 safe 不该进阶段 D 的批量删 | `TierSummary.safe_tool_bytes` 单独算；阶段 D 批量删应只选 `tier = safe && how 含 delete`（决定 57） | 实机：safe 里要用工具清的 0.01 GB（本机没装 conda / pnpm） |
| 14 | 建议 | 微信 radium 整个 safe、没有出处 | 改 review，lose 写小程序本地数据会丢 | 规则测试 |
| 15 | 建议 | 有“其余小文件”时只核最大 50 个文件名 | 回磁盘把全部文件名读一遍（只读名字，读不到算不合格） | 原有的真实目录测试照常通过 |
| 16 | 建议 | 远程 http 会明文传 Key | `llm::check_url`：远程必须 https | 测试：`http://api.example.com`、`http://192.168.1.5` 被拒，localhost / 127.0.0.1 / [::1] / https 放行 |
| 17 | 建议 | battlenet-cache 要先结束 Agent.exe | how_detail 原本就写了；阶段 D 的“需要先关程序”一类留给阶段 D | — |

**产品评审**

| # | 级别 | 问题 | 怎么改的 | 怎么验证 |
|---|---|---|---|---|
| 1 | 严重 | `LocalCache\Local` 这一层（7.56 GB）被说成“Claude 的登录信息” | 新 self 规则 `store-app-sandbox-appdata`（这一层本身就是评审给的那句话）和 `store-app-localcache`（`LocalCache` 这层）；沙箱里按原位置认不出的东西不再沿用包规则，改成“商店应用沙箱里的东西，还没认出是谁的”并把重定向写进线索第一条；`tier_summary` 对沙箱按原位置找规则 | `b3_tests`（沙箱测试改了预期）和 `r1_tests::sandbox_layers_and_build_container`；实机前 50 的第 12、20 行现在是沙箱说明，第 23 行 `DiskWhy-build` 是 rust-target；逐行人工核对见 coverage.md |
| 2 | 一般 | 26 条 safe 没有出处；Raspberry Pi 规则漏旧路径 | 同安全第 12 条；raspberry 补 `Raspberry Pi\Imager\cache` 和 rpi-imager #209；coverage.md 的待办更正 | 同上 |
| 3 | 一般 | 覆盖率口径偏乐观 | CLI 加“叶子覆盖率（按字节）”，`--report` 的 counts 里有 `leaf_bytes_*`，coverage.md 结论表第一列就是它（决定 54） | CLI 单元测试 `leaf_coverage_counts_only_exclusive_bytes`；实机前 50 是 **58.9%**、前 200 是 87.1%（如实写，前 50 低于 80%） |
| 4 | 一般 | C 盘根 20 项里 9 项未知、AppData\Local 120 项里 75 项未知 | 新规则：`C:\Temp` / `C:\tmp`、驱动解压目录 `C:\AMD` / `C:\NVIDIA` / `C:\Intel`、PerfLogs、inetpub（微软 KB5055523 明说别删）、IconCache.db、ConnectedDevicesPlatform、VirtualStore、`AppData\Local\OneDrive`、pip / Yarn 外层、DiskWhy 自己的数据、WebView2 的 `EBWebView`；链接 → “不占空间”；0 字节 → “空的，不占空间”；图例把 < 1 MB 的未认出项合并成“小文件夹 N 个” | 实机：C 盘根这一层 ≥ 1 MB 的未认出 0 项，另有 2 个不到 1 MB 的小文件夹（原来 9 项未知）；AppData\Local 120 项里未认出 ≥1 MB 的 21 项 + 小文件夹 23 个 + 不占空间 22 项（原来 75 项未知）。驱动目录没找到厂商官方说明，所以是 review（决定 58） |
| 5 | 一般 | CacheStorage 算 safe | 同安全第 5 条 | — |
| 6 | 一般 | 汇总卡标题和内容矛盾、副标题截断 | 标题“这些空间分别是什么”；副标题写文件夹名、两行显示、完整说明在悬停；第四张卡写全 | 截图 `phaseB-r1-3` |
| 7 | 一般 | phase-b 没合进 master | **没合**：任务硬约束“main 分支一个字不改”（本仓库是 master）。用 `git merge-tree --write-tree master phase-b`（不动任何分支和工作区）试合：冲突 4 个文件——`Cargo.toml`（两边都加了 windows-sys 的 feature，其中 `Win32_System_Com` 两边都加了）、`src/App.vue`、`src/components/DetailPanel.vue`、`src/styles.css`；`lib.rs`、`api.ts`、`TreemapView.vue`、`win.rs` 能自动合（决定 56） | — |
| 8 | 建议 | PHASE_B.md 过时内容在前面 | 第 1 节改成当前状态，旧的第 1–6 节挪到文末「历史」 | 本文件 |
| 9 | 建议 | 几处文案 | WSL 写明 2.3.11 及以上（GitHub 发布说明 2.3.11 有 “Implement wsl --manage --move”）并教 `wsl --version`；旧版迁移拆成四步、unregister 前先确认导出和目标盘空间；ModelScope 引 issue #845 并建议先用小模型试；squirrel-temp 补全 why / lose；nuget 的 lose 加 restore 提示 | 规则测试 |
| 10 | 建议 | `explain --json` 的 notes 有真实用户名 | 新参数 `--redact`（`--json` / `--report` 里的 `\Users\<名字>` → `\Users\<用户>`；默认关，保留机器可用的真实路径；`--md` 一直脱敏） | CLI 单元测试 `redact_json_replaces_user_in_escaped_paths` |

**冷启动评审**

| # | 级别 | 问题 | 怎么改的 |
|---|---|---|---|
| 1 | 严重 | 前 30 秒看不到任何档位颜色 | 欢迎页和扫描中常驻 `TierGuide`（四种颜色 + 一句话）；欢迎页第一句改成“扫完后每个文件夹都会标上颜色……”，口径说明折叠 |
| 2 | 严重 | 归属条和档位两套颜色撞车，三种灰分不开 | 归属条改成深浅石板色 + 纹理；方块图“读不了”改成白底斜纹；保留改蓝灰 + 淡斜纹 |
| 3 | 严重 | Windows 自带的、DiskWhy 自己的目录也是“我也不确定” | 同产品第 4 条 |
| 4 | 一般 | 未知项说 5 遍不确定、把功课甩给用户 | 1.3 节；读不了的、0 字节的不显示“问 AI” |
| 5 | 一般 | 列表里未知行只写“我也不确定” | 列表这一列现在是“还没认出来，先别动” |
| 6 | 一般 | 色标意思只在悬停里；“可清理 0”像矛盾 | 图例每档一句话常驻，计数写成“这一层 N 项” |
| 7 | 一般 | 同一档四套叫法；“未知”既是档位又是来源；“推测”是琥珀色 | 每档一个名字；来源只留规则库 / 推测（+AI）；推测改成青灰色虚线框 |
| 8 | 一般 | 汇总卡点不了、副标题重名 | 点卡片筛列表（这一层）；副标题写文件夹名并去重 |
| 9 | 一般 | 进度条 30 秒到 97% 后像卡住 | 过 90% 显示“正在收尾”+ 走动条纹（实测第 21 秒到 90%，第 50 秒扫完） |
| 10 | 一般 | 新旧截图混放、旧图露用户名、没有默认视图截图 | 旧的 phaseB-1..7 挪到 `docs/screenshots/old/`，用户名区域打码；新截图在 `docs/screenshots/r1/`，包括默认视图（方块图 + 列表 + 图例） |
| 11 | 建议 | “用这个工具”像按钮 | 改成普通文字 |
| 12 | 建议 | “请看‘该怎么腾’”和栏目名不一致 | 那句提示删了，栏目统一叫“怎么腾” |
| 13 | 建议 | 欢迎卡全是术语 | 同第 1 条 |
| 14 | 建议 | WSL 迁移直接给 unregister | 同产品第 9 条 |
| 15 | 建议 | 保留用灰色像“禁用” | 改蓝灰 + 锁；方块图可选“只高亮可清理” |

### 12.2 怎么验证的

- `cargo test -p diskwhy-core -p diskwhy-cli`：**87 个通过，0 失败，2 个忽略（B-4 时 79 个；忽略的两个要设环境变量才跑）**。`cargo check -p diskwhy`（Tauri 壳）通过。前端 `vue-tsc --noEmit` 通过，`vitest run` 7 个通过。
- 发布版：`npm run tauri build -- --no-bundle` 通过（2 分 40 秒），`cargo build --release -p diskwhy-cli` 通过；这一轮的两个 exe 另存在 `target\tmp\r1fix\`。
- 真实 C 盘（普通权限，只读）：`diskwhy-cli explain C:\ --top 50 / --top 200 --report --md --redact`（现扫 47.7 秒）。结果和逐行人工核对写在 [coverage.md](coverage.md)。全盘各档：可清理 23.83 GB（原 28.96 GB）｜需判断 58.51 GB｜保留 67.54 GB｜规则没覆盖 20.74 GB；护栏改记 1.60 GB（1.57 GB 是名字像 Chromium 缓存但内容不全是缓存格式的文件夹，30 MB 是 Temp 里的文档）；CacheStorage 3.47 GB 从可清理移到需判断。
- 截图：发布版 GUI，用 WebView2 远程调试端口（CDP）驱动界面按钮并截图，视口 1280×800 逻辑像素 × 1.25（和评审一样的尺寸）。截图前把页面文字里的用户名换成 `<用户>`；方块图 canvas 上的用户名和扫描中实时刷新的路径用图片打码（决定 59）。截图期间没有保存设置，`%LOCALAPPDATA%\DiskWhy\settings.json` 不存在。

| 文件（`docs/screenshots/r1/`） | 内容 |
|---|---|
| `phaseB-r1-1-welcome-guide.png` | 欢迎页：第一句说“扫完后每个文件夹都会标上颜色”，下面常驻四种颜色卡，口径说明折叠 |
| `phaseB-r1-2-scanning-guide.png` | 扫描第 8 秒：颜色卡还在，进度 61% |
| `phaseB-r1-2b-finishing.png` | 第 21 秒过 90%：“正在收尾”+ 走动条纹 |
| `phaseB-r1-3-root-default-view.png` | 扫完的默认视图（方块图 + 列表）：中性归属条、“这些空间分别是什么”四张卡（可清理 23.83 GB，副标题“Temp 5.13 GB、Cache …”）、护栏说明、图例常驻一句话 |
| `phaseB-r1-4-root-highlight-safe.png` | 勾“只高亮可清理”：根层没有大块可清理的，整张图变淡（可清理的都在深层小块里） |
| `phaseB-r1-5-root-list-inetpub.png` | C 盘根列表：inetpub 是“保留”并写明微软说别删；根这一层没有未认出的项 |
| `phaseB-r1-6-local-temp-guarded.png` | `AppData\Local`：Temp 因为里面有文档被标“需判断”；图例“未认出 21 项、另有 23 个小文件夹、不占空间 22 项” |
| `phaseB-r1-7-filter-safe.png` | 点“可清理”卡：列表只剩这一层的 16 个可清理项 |
| `phaseB-r1-8-unknown-detail.png` | 未认出的 OpenAI：线索第一条“很可能是「OpenAI」/「Codex」相关的程序文件（.exe / .dll 占 95%）” |
| `phaseB-r1-9-sandbox-localcache.png` | `Claude_…\LocalCache`：Roaming / Local / LocalLow 都是“商店应用沙箱：这个应用以及它启动的程序写到 AppData 的东西，被 Windows 挪到了这里” |

### 12.3 没做 / 没验证的

- **没有合并到 master**（决定 56）：阶段 D 的删除按钮会不会按新的 tier / 护栏正确显示，没有验证；合并时要手动解决 4 个文件的冲突，并让阶段 D 的批量删除只选 `tier = safe` 且 `how` 含 `delete` 的项。
- **前 50 的叶子覆盖率 58.9%，低于 80%**：前 50 行里 17 行是容器，容器自己独占的约 69 GB 只有“点进去看”这种说明；前 200 是 87.1%。没有为了抬高这个数字去改口径。
- 用户数据陷阱的签名是按经验挑的（聊天文件夹名、conda-meta、虚拟磁盘扩展名、.git/.svn/.hg、Office/WPS/PDF 扩展名）；`package.json`、`Cargo.toml`、`*.sln` 没算进去（决定 43）。被合并进“其余小文件”的文件看不到名字，里面的文档认不出。往下找最多看 30,000 个节点。
- JetBrains “手动删 caches 会丢 Local History”仍是根据官方页推断，没实测。显卡驱动解压目录、C:\Temp 等新规则没有厂商官方出处，所以都是 review / keep。12 条 safe 规则只有本机观察（名单在测试里）。
- AI 这一轮没连真实模型重跑（设置保持关闭）；新的请求体只用测试里的假 oracle 和本地假服务器验证过。
- “问 AI”在后端只挡了“不是未认出”的项；读不了、0 字节的只在前端隐藏按钮。
- `SHGetKnownFolderPath` 只读了 6 个个人文件夹；OneDrive 仍靠环境变量。
- 截图没有窗口标题栏（CDP 只截页面）；方块图里保留块的“锁纹”在大容器上会铺满，视觉上偏满。
- 汇总卡的筛选只作用于当前这一层列表（不是全盘清单）。

### 12.4 下一阶段从哪接

1. 用户确认后合并 phase-b → master（冲突见 12.1 产品第 7 条），合完在 master 上重新跑全部测试、发布版构建、截图、覆盖率，并核对阶段 D 的删除按钮：keep 没有按钮；护栏降级后的 review 不进批量删；批量删只选 safe + delete。
2. 覆盖率：给前 50 里的容器补“里面主要是什么”的动态说明，或补更具体的规则，把叶子覆盖率提上去。
3. 让第 2 轮评审复跑安全探针 `D:\desk\claudex\_ops\S22\r1-safety-probe`（它依赖的 API 没变）。

---

## 历史：第一次做阶段 B 时的记录（2026-09-23 上午，已过时）

> 下面是原来的第 1–6 节，原样保留作对照。里面的规则数（179 条）、测试数（47 个）、“启发式永不 safe”、LLM 发 9 个字段、截图文件名等都已经过时；截图已挪到 `docs/screenshots/old/`。

### 旧 1. 做了什么

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

### 旧 2. `cargo test`（core + cli）

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
| `llm_request_body_has_no_file_content_and_redacted_path` | 文件里写机密串，序列化请求体里没有；`C:\Users\<用户>` 已变 `~`；只有 9 个字段 |
| `llm_end_to_end_against_local_fake_server` | 起一个本地 TCP 假 OpenAI 服务，抓**真实发出的 HTTP 字节**：`POST /v1/chat/completions`、带 Bearer、不含机密串、用户名已脱敏、结果 review+Llm |
| `llm_network_failure_is_graceful` | 连 127.0.0.1:9 返回 Err 不 panic；未启用时给“去设置”提示 |

---

### 旧 3. CLI 对真实 C 盘：`diskwhy-cli explain C:\ --top 50`

```
DiskWhy 解释：C:\
扫描 40.9 秒 | 卷已用 183.33 GB | 规则 179 条（含用户规则，加载+注册表共 15 ms）| 注册表已安装软件 260 个（读取 13 ms）| 解释前 50 大目录用时 10 ms
  #          占用  档位      来源                                  路径
  1   116.67 GB  保留      规则 users-root                       C:\Users
 10    11.99 GB  保留      规则 winsxs                           C:\Windows\WinSxS
 13    10.28 GB  需判断     规则 recycle-bin                      C:\$Recycle.Bin
 28     7.48 GB  保留      规则 docker-data-vhdx                 C:\Users\<用户>\AppData\Local\Docker\wsl\disk
 32     6.30 GB  需判断     启发式 media-files                     C:\Users\<用户>\AppData\ACLOS
 38     5.29 GB  保留      规则 wsl-distro-vhdx                  C:\Users\<用户>\AppData\Local\wsl\{d1f9e0f8-…}
 40     4.93 GB  需判断     规则 feishu-larkshell                 C:\Users\<用户>\AppData\Roaming\LarkShell
 45     4.53 GB  需判断     规则 nvidia-app-updates               C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts
 48     4.20 GB  可清理     规则 user-temp                        C:\Users\<用户>\AppData\Local\Temp
 49     3.98 GB  需判断     规则 chrome-on-device-model           C:\Users\<用户>\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel
 …（完整 50 行见 coverage.md）
统计（前 50 大目录）：规则命中 47（精确 34 + 继承上层 13）| 启发式 3 | 未知 0 | 规则+启发式覆盖率 100.0%
全盘按规则库归档（153 ms）：可清理(safe) 29.04 GB | 需判断(review) 60.89 GB | 保留(keep) 75.13 GB | 规则未覆盖 28.44 GB | 命中规则 93/179
```

`--top 200`：规则 172（精确 80 + 继承 92）| 启发式 19 | 未知 9 | 覆盖率 95.5%，解释 200 个目录用时 43 ms。未知项和下一批规则的待办见 coverage.md。

开发过程中实跑发现并修掉的两个问题（都补了测试）：
1. 注册表归属把 `ProgramData\NVIDIA Corporation\NVIDIA App` 说成了“NVIDIA Nsight”——厂商文件夹名 `NVIDIA` 前缀匹配到了第一个 NVIDIA 软件。改为：多层文件夹名先精确匹配，前缀匹配只接受唯一结果，否则只说到厂商。
2. `scope = "self"` 的容器规则在汇总和“其余小文件”上认领了容器里的散文件，已改为只说明文件夹本身。

---

### 旧 4. GUI（`target\release\diskwhy.exe`，`npm run tauri build -- --no-bundle` 构建，普通权限）

截图方法同阶段 A：PowerShell 用 UI Automation 的 `InvokePattern` 点按钮/行、`ValuePattern` 填输入框、`ScrollPattern` 滚动详情面板，`PrintWindow(PW_RENDERFULLCONTENT)` 抓窗口。GUI 内扫描 C 盘用时 46.6 秒。

| 文件 | 内容 |
|---|---|
| `screenshots/old/phaseB-1-list-root.png` | 扫描完成：左上归属条，右上“本次扫描能腾多少”汇总卡（可放心清理 29.05 GB / 需要你判断 62.35 GB / 建议保留 75.13 GB / 规则还没覆盖 28.44 GB）；`C:\` 列表每行左侧档位色条 + 档位小标签 + 一句话“这是什么”；右侧 C 盘根目录的说明 |
| `screenshots/phaseB-2-list-and-safe-detail.png` | `AppData\Local` 列表（绿/黄/红混排）；选中 `Temp`：大字“可以放心清理” + 六项 + 该怎么腾 |
| `screenshots/phaseB-3-keep-migrate-docker.png` / `phaseB-3b-…-howto.png` | `Docker\wsl\disk\docker_data.vhdx`：“建议保留，别删”，能腾出“不能删；迁到别的盘可让 C 盘腾出约 7.48 GB”，该怎么腾里写 Docker Desktop 的 Disk image location 和 prune；来源 `规则库：docker-data-vhdx（分组 dev-caches）`，两个参考链接（带“复制链接”） |
| `screenshots/phaseB-4-unknown-clues.png` | `AppData\Local\OpenAI`：灰色“未知”，“我也不确定，这是我看到的线索”列出最大子项、扩展名分布（.exe 93%）、文件数、修改时间、在 AppData 下、注册表里找不到“OpenAI”；下面是“问 AI（先去设置里启用）”按钮和发送内容说明 |
| `screenshots/phaseB-5-settings-test-fail.png` | 从未知项的按钮进入设置页；点“填入本机 Ollama”后测试连接，本机 Ollama 返回 `HTTP 404 model 'qwen2.5:7b' not found`——**错误显示在对话框里，没有弹窗** |
| `screenshots/phaseB-6-settings-test-ok.png` | 模型改成本机已有的 `qwen3-vl:4b-instruct-q4_K_M` 并启用，测试连接：“连接成功，用时 7.1 秒，模型回复：你好” |
| `screenshots/phaseB-7-ask-ai-result.png` | 保存后对 `OpenAI` 点“问 AI 这是什么”（3.95 秒返回），黄色虚线框“AI 推测 · 需要你来判断”，可信度 80%，来源“AI 推测（模型 qwen3-vl:4b-instruct-q4_K_M）”。这是真实的本机模型回答（模型只看到名字和大小，猜得比较泛），也说明了为什么 AI 结果只能当 review |

截图结束后已删除测试时写入的 `%LOCALAPPDATA%\DiskWhy\settings.json`，恢复默认关闭。

---

### 旧 5. 查过的官方文档

- Win32：`RegGetValueW`（`pcbData` 含结尾 NUL、`ERROR_MORE_DATA` 时返回所需大小、不带 `RRF_NOEXPAND` 时自动展开 `REG_EXPAND_SZ`）；“Accessing an Alternate Registry View”（用 `KEY_WOW64_64KEY` / `KEY_WOW64_32KEY` 分两遍枚举、子键要用同一个标志、`Wow6432Node` 是保留名不要直接用——实现里 HKLM 32 位那一遍就是用 `KEY_WOW64_32KEY` 打开同一路径，而不是拼 `WOW6432Node`）。
- ureq 3.4.2：docs.rs 首页 + 本地 crate 源码（`Agent::config_builder`、`timeout_global`、`http_status_as_error`、`TlsConfig::provider`、`native-tls` feature 定义、默认 provider 是 Rustls）。
- WSL basic-commands（export/import/unregister）、本机 `wsl --help`（`--manage --move`）、Ollama FAQ（`OLLAMA_MODELS`、默认路径）、Docker Desktop settings、DISM WinSxS 清理、powercfg 休眠、Delivery Optimization PowerShell cmdlet、conda clean / manage-environments、HF manage-cache 等（完整列表即规则里的 57 个 refs，全部 200）。
- Tauri 2：沿用阶段 A 查过的 calling-rust（`State`、async command 返回 `Result`）与 Channel 用法，本阶段没有新增插件和权限。

---

### 旧 6. 没做到 / 没验证的

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

