# 阶段 J 计划：用户第一次真用之后的问题（2026-10-06）

规划 / 调查：Opus 5.5（主会话）。疑难问题按用户要求请 Fable 子代理把关。

## 0. 事实（真机调查，2026-10-06）

**这次清理（actions.log，批次 20261006-115208-389a01）**
- 本地时间 05:52:08 开始，06:19:47 最后一条记录；共 323,851 条：回收成功 299,081、删空文件夹 23,752、跳过 565、回收失败 444（“回收站操作被中止”）。日志记录的成功字节 31.5 GB。
- **没有 summary 记录 = execute 没有返回**。`uv\cache` 做了 109,825 个，还剩 81,233 个文件（1.47 GB）没动 → 程序在执行中途退出。
- Windows 事件日志里没有 diskwhy.exe 的 Application Error / Hang / WER 记录（这台机器近期任何程序都没有 Application Error 事件，不能据此排除原生崩溃）；06:39 有两条显卡 LiveKernelEvent 141（与 06:19 不重合）。WebView2 的 Crashpad\reports 为空。**根因未定**。
- 速度：前 15 分钟 4 个清单项并行，~16k–24k 条/分钟；第 16 分钟起只剩 `uv\cache` 一个清单项，**单线程** ~5k 条/分钟（85 个/秒），这 12 分钟只多了 0.3 GB（uv 缓存 11 万个小文件、不少是硬链接 → bytes=0）。=“卡在 28.7 GB”的观感。
- 进度条 = `done / total_entries`（按文件数）；执行中 `close()` 直接 return，没有停止按钮；后端 `ExecOptions.cancel` 存在但 Tauri 层传的是 None。
- **回收站**：`C:\$Recycle.Bin\<SID>` 现在 260,497 项、24.39 GB（C 盘回收站上限设置 12,238 MB，实际已超）。C 盘剩余 7.2 GB。确认弹层写的是“清空回收站后 C 盘剩余 x → y”，完成页写“已进回收站”，但用户只看到大数字。
- “改放 D:\”：回收站所在卷可用 < 10% 时出现，点了 = 策略换成“隔离到 D:\”重新预检。**D:\ 根目录 ACL 只给 Everyone 读执行**（icacls 实测），普通用户建不了 `D:\DiskWhy…` → 预检给阻止性警告，按钮不可用 = 死路。
- 30.1 → 29.3 GB：首页数字是扫描时各组 `deletable_bytes` 之和；弹层先显示同样的估算，逐个文件检查完换成真正能删的（扣掉正在使用 / 30 分钟内改过 / 云文件 / 没权限 / 扫描后已变化的），所以变小。阶段 G 文档已记为“数字会跳一下”，界面没解释。
- 启动即扫 C:，界面上没有换盘入口（`scanRoot` 只在 init 里设成 C:）。
- 方块图“选不到 C:\ 和 Users”：`openDir` → `defaultFocus` 用 `collapseChain` 自动把右边列表跳到最大子文件夹的最深“外壳”（Users → king）。
- 方块图灰色 = `keep`（先别动）；斜条纹（只在 C 主题）= `mix`（认不出来 / 合并的“其他 N 项”）。没有图例。用户当前主题 = C。
- 窗口：`tauri.conf.json` minWidth 1100 / minHeight 720；竖屏副屏宽度常 < 1100 逻辑像素。

**用户问的东西**
- 录像：C 盘 `C:\Users\king\AppData\ACLOS\aclos-highlight\` = 《无畏契约》“无畏时刻”自动录像，6.3 GB（7 月 0.8 GB / 8 月 1.0 GB / 9 月 4.5 GB）；D 盘 `D:\valorant_game\Tencent Games\wonderfulVideos6192787854092623945\` 147 GB（3–10 月，含 5 月 1.6 GB）。C 盘没有 5 月的视频。
- 飞桨：`C:\Users\king\.paddlex\official_models\PP-OCRv6_medium_det|rec`，0.13 GB，2026-07-17 19:30 下载；来自 `D:\ideas\SearchMyAlbum`（拾光索，本地照片标注引擎）的 OCR 步骤，conda 环境 `D:\conda\envs\album`（7/15 建，paddleocr 3.7.0 / paddlex 3.7.2 / paddlepaddle 3.3.1）。
- Chrome AI：`OptGuideOnDeviceModel\2025.8.8.1141\weights.bin` 4.07 GB，2026-05-07 Chrome 自己下载（Gemini Nano），不是用户操作；关法：Chrome 设置 → 系统 → “设备端 AI / On-device AI” 关掉后再删，否则会重新下载。

## 1. 要改的（按文件归属分三条线）

### R：后端（crates/、src-tauri/src/）
- R1 执行提速：清单项内部也并行（大项切块分给多个工作线程），不再“最后一个大项单线程跑 12 分钟”。
- R2 删除方式：见 §2 待定。
- R3 停止：`cancel_execute` 命令 + 执行里每个文件 / 每批检查；停下后返回 `cancelled = true` 的报告（已处理的照常记账，剩下的原样不动）。
- R4 预计时间：预检报告给 `est_secs`（按文件数 × 每种方式的单文件成本 + 跨盘按字节吞吐，常数用本机实测标定）；执行进度带 `elapsed_ms`、`total_bytes`，前端按字节画条、按速率算剩余时间。
- R5 说实话：执行前后读源盘可用空间，报告 `freed_bytes`（实际多出来的空间）；按去向分别统计字节（直接删 / 回收站 / 隔离）。
- R6 闪退取证：panic hook → `%LOCALAPPDATA%\DiskWhy\crash.log`；未处理的原生异常 → minidump；执行开始写 `start` 记录；下次启动发现最后一批没有 summary → 前端提示“上次清理被打断，处理了 N 个”。
- R7 隔离区位置：盘根建不了时自动退到该盘上用户可写的位置（或让前端请用户挑），不再出现点了“改放 D 盘”却被拦死的情况。

### F1：清理流程前端（ConfirmDialog.vue、store.ts 执行部分、api.ts、labels.ts）
- 进度条按字节；下面一行“还剩约 N 分钟 · 剩 N 个文件”；“停止”按钮。
- 弹层：执行前显示“约 N 分钟”；数字比首页少时，跳过一行带字节和原因（“跳过 539 个 · 0.8 GB · 正在使用”）。
- 完成页：大数字 = C 盘实际多出来的空间；分去向的小字；放进回收站的部分明说“在回收站里，C 盘还没变大”。
- 上次被打断的提示。

### F2：布局与说明前端（HomePage、TitleBar、CleanupPage、ExplorePage、TreemapView、SettingsPage、QuarantinePage、样式、tauri.conf.json）
- 换盘：首页盘符做成可点的小下拉（C 盘 ▾ → D 盘…），选了就扫那个盘；仍然一打开就扫 C。
- 自适应：minWidth 降到能放进竖屏；各页在窄 / 竖窗口下改成上下排（首页大数字在上、容量柱横放或在下；清理页左侧分类变横向标签；分析页方块图在上、列表在下）。在 1440×900、1100×720、864×1400（竖屏 125%）、720×1200 下截图检查。
- 方块图：点 C:\、Users 等任何一层都能选中并在右边列出；面包屑每一级可点；加图例（悬停 / 点“图例”出现，不常驻文字）。
- 设置：“AI 猜文件夹”改名 + 一句话说明（点 ? 才展开）+ 服务商预设（DeepSeek / 通义千问 / Kimi / OpenAI / 本机 Ollama）+ 密钥；“扫描存档”改成“保存 / 打开扫描结果”；“隔离区”改名并说明（删掉的先放这里 N 天，可放回）。

## 2. 待定（请 Fable 把关）
- “可放心清理”（safe 档，自动重建的缓存）是否默认**直接删除**（不进回收站）——回收站在同一个盘，不腾空间、还有容量上限、逐个回收极慢；“需你判断”默认隔离到其他盘（可找回、真腾空间），没有其他盘时进回收站。
- 回收站方式下是否按“整个子文件夹”回收（预先逐个复核指纹后整夹送进回收站）以减少几十万条回收记录。
- 进度条按字节 vs 按文件数：时间和文件数近似线性，和字节不线性（同盘回收 / 删除是改名或删记录，和文件大小几乎无关；跨盘隔离才和字节有关）。

## 3. 决定（Fable 审查，2026-10-06；实现以此为准）

**0. 漏掉的 P0**：`TitleBar.vue` 的 ✕ 无条件 `w.close()`，而执行中弹层不能关、没有停止按钮——“程序消失”最可能是用户点了 ✕（或 Alt+F4），进程退出、没有 summary（和“没有任何崩溃记录”吻合）。修法：执行期间拦截关窗（Rust 侧 `WindowEvent::CloseRequested` 在 `acting` 时 `prevent_close` 并发事件给前端；前端闪一下「停止」）；actions.log 写 `start` 记录 + 每 30 秒一条 `heartbeat`；下次启动发现最后一批没做完 → 一行「上次清理中断，已处理 N 个」。minidump 放到最后做。

**1. 删除方式：改良 B**
- `safe` 档**直接删除**（`DeleteFileW`，过护栏 9 重校验后）；`review` 档去用户选的去向。理由：safe 按定义可自动重建，回收 30 万条缓存既慢（85/s）又不是真撤销——回收站 24 GB 已超 12 GB 上限；Storage Sense 本来就直接删。
- 「删到哪里」下拉（只管 review 档）：`D 盘暂存区`（默认，当存在可写的其他固定盘且空间够）→ `回收站`（没有其他盘时默认）→ `直接删除`（review 仍走已有的勾选确认）→ `其他文件夹…`。**唯一例外**：用户**明确选**「回收站」时 safe 也进回收站。弹层去向行写两段短字：`可放心清理 直接删 · 需你判断 暂存到 D 盘`。
- 后端：`Strategy` 加 `Permanent`；`Plan` 加 `safe_direct: bool`；按 `item.tier` 选操作；`ExecReport` 按去向分计 `{permanent, recycle, quarantine}` 字节 / 条数。9 条护栏原样不动（都在去向之前执行）；护栏 8（回收站空间提示）只在有东西进回收站时触发。
- 如果某条 safe 规则命中了用户数据——改那条规则，不改策略。

**2. 暂存区落点（X:\ 根不可写时）**，按序：① `X:\DiskWhy暂存区` 已存在且可写 → 用；② 试建 → 成功就用；③ `quarantine-roots.json` 里该盘以前用过的位置仍可写 → 用；④ 弹层里一个「授权」按钮，UAC **一次**：`diskwhy.exe --mkdir-quarantine X:\` 提权建目录并 `icacls … /grant <当前用户>:(OI)(CI)M`；⑤ 拒绝 / 失败 → `选文件夹…`。**不**自动挑某个用户文件夹。`list_batches` 同时认旧名（`DiskWhy-Quarantine` 等现有目录名）。非管理员账户 ④ 必失败，直接给 ⑤。

**3. 提速**
- 所有未拦下的清单项的条目切成 256 条一块，进**同一个工作队列**（各项交错，避免最后一个大项单线程），线程数 `clamp(cpu, 4, 16)`（回收模式每线程 STA）；每项有剩余块计数，做完最后一块的线程负责该项的空目录删除 + manifest 落盘（另外每 10 秒 flush 一次 manifest 防中断）。
- 整夹回收（只在回收站模式）：门槛严——子树全部 `Delete` 且逐条重校验通过、没有链接 / 云目录、入队前重新 `list_dir` 名字集合完全一致、子树 ≤ 1 GB（避免 `FOF_WANTNUKEWARNING` 弹窗）→ 一个 `DeleteItem(dir)`；任一不满足就逐文件。日志仍逐文件记。
- 跨盘拷贝改 `CopyFileExW` 带 `pbCancel`，停止能中断大文件。`cancel_execute` 命令 + `AppState` 存 `Arc<AtomicBool>` 传进 `ExecOptions.cancel`。

**4. 进度**：按**估算成本加权**：每条 cost = 单文件常数（按操作）+ 字节 / 吞吐（只跨盘）；进度条 = done_cost / total_cost，单调。常数初值：直接删 0.1 ms、回收 2 ms、同盘改名 0.2 ms、跨盘 1 ms + 150 MB/s；每次执行后把实测写 `%LOCALAPPDATA%\DiskWhy\perf.json`，预检 `est_secs` 用它。弹层按钮旁 `约 2 分钟`（< 15 秒显示 `几秒`）；执行中大数字 = 已处理 GB，下一行 `剩约 2 分 · 停止`（速率取 5 秒滑窗，< 15 秒显示 `快好了`）。进度事件改为**每 200 ms** 发一次，带 `elapsed_ms`。

**5. 完成页**：大数字 = 执行前后 `GetDiskFreeSpaceExW(源盘)` 实测差值（≥ 0），标签 `C 盘多出`（盘符按实际）；悬停数字显示「按清理前后剩余空间实测」。小字只列非零：`直接删除 28.1 GB` / `暂存到 D 盘 2.0 GB` / `回收站 0.4 GB · 还没腾出` / `跳过 565 ›`（展开带字节与原因，也解释 30.1→29.3）。停止后标题 `已停止`，同样实测。P2：回收站部分 > 0 时加「清空回收站」两步确认（`SHEmptyRecycleBinW`）。

**6. 换盘**：首页右栏 `C 盘` 标签变成 `C 盘 ▾` 下拉，只有固定盘 > 1 个时出现；菜单行 `D 盘` + 小字剩余 GB；选中 → `scanRoot = …; doScan()`。启动逻辑不变（一打开就扫 C）。所有硬编码 `C 盘` 改成从 `scanRoot` 推导；“改放 D:\” 芯片改 `改放 D 盘`，走第 2 条的回退链。

**7. 自适应**：`tauri.conf.json` minWidth 720 / minHeight 640。断点：`narrow` = `(max-width: 1099px), (max-aspect-ratio: 1/1)`；`short` = `(max-height: 759px)`。narrow 时：首页大数字区在上，容量柱改**横向**（`CapacityColumn` 加 `horizontal`，`capacity.ts` 的一维防重叠复用），`--big: min(220px, 24vw, 22vh)`；清理页左栏分类变横向可滚 chip 行，表格只留 名字 / 标签 / 大小 / 勾选；分析页改纵向，方块图约 45% 高在上、列表在下，< 820 宽时副栏换行；确认弹层 `width: min(540px, 100vw - 32px)`；设置页主题预览缩到 100px。验证尺寸：1440×900、1100×720、864×1400、720×1200、720×640。

**8. 设置**
- `AI 识别`（开关），? →「认不出的文件夹问 AI 猜一猜。只发名字和大小，不发内容；结果仅供参考。」
- `扫描结果`：按钮 `保存` / `打开`，? →「存成文件，下次不用重扫。」
- `暂存区`（界面文案里的“隔离区”全部换成“暂存区”：页名、`暂存到 D 盘`、`已暂存`；后端标识不改），? →「删掉的先放这里 7 天，可以放回原处。」
- 服务商预设分段 `DeepSeek · 通义千问 · Kimi · OpenAI · 本机`，点一下填好地址 + 模型（**实现时按各家官方文档核对 URL 和模型名**）；每个预设配 `获取密钥 ↗`。DeepSeek 排第一，本机最后、悬停「本机小模型猜得不准」。密钥加密（DPAPI）是 P2。

**9. 图例与选中**：分析页加 `图例` 按钮，弹出 5 行，色块**复用方块的 CSS 类**（C 主题斜纹就是真斜纹）：可放心清理 / 需你判断 / 先别动（系统·程序·你的文件）/ 认不出来（含“其他 N 项”）/ 扫不到（无权限·系统保留）。mix 块悬停没有说明时兜底「认不出来，点开看线索」。
选中修法：面包屑第 0 级 → `openDir(root, root)`；第 i 级 → `openDir(parent(bc[i]), bc[i])`；`enterBlock(p)` → `openDir(p, p)`；`setFocus(path == listing 根)` → focus = listing、focusKey = null；`defaultFocus` 自动下钻只在第一次进入（`listing === null`）时用。

**10. 其他风险**
- 硬链接陷阱：跨盘暂存会把 `bytes=0` 的硬链接整份拷到 D:（占空间又不腾空间）。跨盘暂存时链接数 > 1 的文件跳过（新跳过原因「硬链接，删了不腾空间」），目的盘估算用 Σ size。
- 回收站超上限会被 Windows 静默清掉最早的：回收模式在去向上悬停说明；P2 读 `BitBucket\Volume\<guid>\MaxCapacity`，超限就提示。

## 4. 交接（2026-10-06，额度用完时）
- 工作流 `diskwhy-phase-j-build-1`（R 后端 ∥ F2 界面，之后 R harden）跑了约 20 分钟后被**主动停止**（额度用完）。工作区里可能有**未提交、不完整**的改动（分支 phase-j，基于 master 6a8b01c）。下一次先 `git status` / `git diff` 看清楚，再决定是在此基础上续做还是 `git checkout -- .` 重来（此前没有任何提交）。
- 脚本：`C:\Users\king\.claude\projects\D--desk-claudex-13-diskwhy\13680466-cb4e-4db2-8449-e9655c7ca804\workflows\scripts\diskwhy-phase-j-build-1-wf_17573a2e-a12.js`；闪退分析在 `docs/PHASE_J_CRASH.md`（harden 步骤会读它）。
- 剩下：R、F2 两条线做完 → F1（确认弹层 / 停止 / 预计时间 / 完成页 / 标题栏拦关窗 / 上次中断提示，按 R 的接口）→ 发布版构建 + 真机截图（多种窗口尺寸）+ 对抗式审查 → 复制到 `D:\desk\claudex\13\DiskWhy-app\`。

## 5. 交接（第二次，2026-10-06，额度又用完）
- 工作流 build-1 已**完整跑完**：R（后端引擎 / 停止 / 进度 / 实测腾出空间 / 暂存区回退链 + UAC / 拦关窗事件）、R harden（读 PHASE_J_CRASH.md 补取证）、F2（换盘 / 自适应 / 分析页选中 / 图例 / 设置改名与预设）。三个代理的完整报告（含前端要对接的接口清单、测试结果、基准数据）存在 `docs/PHASE_J_build1_reports.json`。已作为一个 WIP 提交放在 phase-j 分支上，**还没人复核**。
- 后端自报：cargo test 107 passed；基准 直接删 200k 文件 12 秒（16,724 个/秒，旧引擎约 185 个/秒）。
- 剩下：F1（确认弹层：去向下拉、预计时间、按耗时的进度条、停止、完成页实测“C 盘多出”、跳过 › 解释 30.1→29.3、改放 D 盘走 prepare_quarantine / elevate_quarantine、close-blocked 事件、上次中断提示、TitleBar 页名改“暂存区”）→ 发布版构建 + 真机多尺寸截图 → 对抗式审查（删除护栏、并发 / 停止、界面）→ 复制到 DiskWhy-app。
