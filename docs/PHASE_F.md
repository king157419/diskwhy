# 阶段 F 记录：三主题界面重做 + 后果类型字段

日期：2026-09-30　机器：Windows 11 Pro 10.0.26200，2560×1600 / 125% 缩放，普通用户权限
构建目录：`CARGO_TARGET_DIR=C:\Users\king\AppData\Local\DiskWhy-build\target`
设计依据：`docs/mockups/v2/`（用户确认的 12 张稿子 + 可切主题的 HTML 原型）、`docs/design-research.md`

## 1. 改了什么

**规则层（Rust）**
- 规则新字段 `lose_kind`（`irreversible | repair | unsure | reinstall | relogin | feature | redownload | slower`，声明顺序即严重程度）。启动校验：写了不认识的值 = 错误；`tier = "review"` 必须写（`safe` / `keep` 可不写）。
- `Explanation.lose_kind` 以 snake_case 字符串传给前端。规则降档（safe 落在个人文件夹 → review）、未知、AI 推测一律 `unsure`；启发式按名字逐个指定（`heuristic_lose_kind`：rust-target / dotnet-build → slower，node-modules / 模型 / 安装包 → redownload，SDK / 运行时 → reinstall，驱动包 → repair，录像 / 媒体 / 模拟器 → irreversible，虚拟机 / 日志 / 临时文件 / 注册表归属等 → unsure；“几乎全是某某”的容器跟着内层走）。**前端不按关键词猜**。
- 72 条 review 规则逐条人工标注（按 `lose` 文案取最坏后果；README 表里的例子全部照办）：unsure 18 · irreversible 14 · reinstall 11 · redownload 9 · relogin 9 · repair 8 · feature 2 · slower 1。其他规则文字一个字没改。
- 新增 3 条规则（本机原来是“未知”）：`root-tmp-dir`（C:\tmp）、`root-temp-dir`（C:\Temp），review / unsure / 只能手动；`amd-installer-files`（C:\AMD，本机里是 Chipset_Software 的 Binaries / Packages / Logs），review / repair / 可删。verified_on = 2026-09-30。现在 review 规则共 75 条。
- 新模块 `explain::candidates`：扫描后按规则 / 启发式把“能清理 / 需要你判断”的东西分组（`ScanSummary.candidates`），清理页直接列出来。只在规则**精确命中**处出候选（预检的规则一致性能过）；里面套着更严格规则（keep、不能直接删、safe 里套 review）的不整个删、只记自己那部分；保护路径拦下的标成不能勾；商店应用重定向的 LocalCache 不列；启发式只认可删的 rust-target / node-modules / dotnet-build。每组有短名字（内置规则有一张界面短名表，用户规则从 owner + 分类拼）。
- `TreemapNode` 加 `lose_kind`、`category`（方块图悬停卡片、列表灰色类别用）。
- 界面偏好：`get_ui_prefs` / `set_ui_prefs` → `%LOCALAPPDATA%\DiskWhy\ui.json`（`{"theme":"A|B|C"}`）。
- CLI `explain` 多打印一段清理候选（组数、两档合计、前 N 组），JSON 里带 `candidates`。

**前端（Vue）**
- 设计令牌 `src/styles/tokens.css`：`[data-theme="A|B|C"]` 定义全部颜色 / 圆角 / 阴影 / 字体；C 的 2px 描边和硬投影由 `--bw` / `--shadow-*` 吸收，三主题共用同一套布局。
- 自绘标题栏（窗口 `decorations: false`，能力里加了最小化 / 最大化 / 关闭 / 拖动权限），首页是品牌，其他页是「‹ 页名」。
- 首页：一打开就自动扫描 C 盘（没有“开始”按钮），扫描时同一个布局显示已扫描 GB、文件数、当前路径和“取消”，容量柱按已扫描 / 未扫描实时变化；扫完是大数字 + 分类小字 + 「清理 X GB」+「看看都是什么」，右边竖向容量柱（剩余 / 可放心清理 / 需你判断 / 其他，每段右边直接写名字和数字、虚线“清理后剩余”），底栏右边三个小色块切主题。
- 容量柱标签防重叠（`src/capacity.ts`）：非空段至少 14px（从高的段按比例扣）、标签相邻至少 36px 上下挤开并限制在柱内、离开段中点就画折线引线。
- 清理页、看看都是什么（方块图改成 DOM 绘制，两层，外壳文件夹合并成 “Users \ king”）、确认弹层、隔离区、设置（主题三个小预览 + AI 设置 + 数据），均按稿子重做。
- 确认弹层：需你判断的项置顶高亮 + 一个后果标签，其余按分类一行（可展开）；「跳过 N 个 · 正在使用 ›」展开看原因；只有含需你判断的项时才出现勾选，没勾时按钮不可用；回收站 / 隔离到某盘是清单标题右边的小下拉，回收站空间告急时下面出现一行「回收站在 C 盘，不腾空间 [改放 D:\]」；回收站模式下总数旁注明“清空回收站后”。
- 主题：A 浅色 / B 深色 / C 趣味；第一次启动按系统深浅色给默认（深 → B，浅 → A）并存进 ui.json，之后以 ui.json 为准；localStorage 只做防闪烁镜像；同时调用 `window.setTheme`（A / C 浅、B 深）。
- 删掉不再使用的组件：AppSidebar、AttributionBar、DetailPanel、DirList、DiskRing、PreviewDialog、ScanControls、ScanProgressPanel、TierSummaryCard、ui/*、OverviewPage、AnalysisPage，以及 action.ts / format.ts / pseudo.ts。
- 仅开发用：`npm run dev` 在浏览器里（不在 Tauri 里）时装一个假后端（`src/dev/`，已 gitignore，正式构建里不包含）。

## 2. 字体（全部打包进程序，运行时不联网）

见 `src/assets/fonts/LICENSES.md`，每个字体附 OFL 全文；子集脚本 `scripts/subset-fonts.py`。

| 文件 | 字体 | 用途 | 大小 |
|---|---|---|---|
| `NotoSansSC-DiskWhy.woff2` | Noto Sans SC 可变字重（OFL） | 三主题中文界面；子集 = 界面文案 + 规则文案 + GB2312 一级 3755 字 + 标点，共 3897 字 | 1983.5 KB |
| `DiskWhyKai-Bold.woff2` | 霞鹜文楷 Bold v1.330 子集，按 OFL 改名 “DiskWhy Kai” | C 主题中文标题 / 标签；只含界面文案用到的 730 字 | 309.1 KB |
| `Geist-latin.woff2` / `Geist-latin-ext.woff2` | Geist 可变（OFL，来自 @fontsource-variable/geist 5.3.0） | 西文、数字、B 主题大数字（200） | 28.7 + 16.1 KB |
| `GeistMono-latin.woff2` | Geist Mono 可变（OFL） | 路径 | 22.6 KB |
| `InstrumentSerif-digits.woff2` / `-Italic-digits.woff2` | Instrument Serif（OFL），只留数字 / GB / . , 等 | A 主题大数字、斜体 GB | 4.5 + 4.7 KB |
| `BricolageGrotesque-digits.woff2` | Bricolage Grotesque 可变（wght / wdth / opsz，OFL），同上 | C 主题大数字（800 / wdth 75） | 60.7 KB |
| **合计** | | | **2,488,064 字节 ≈ 2.37 MB**（目标 < 3 MB） |

`@fontsource-variable/geist*` 两个 npm 依赖已移除（文件直接放进 `src/assets/fonts/`）。霞鹜文楷上游 TTF 的 format 4 cmap 子表是截断的，子集脚本先只保留完整的 format 12 子表再补一个 format 4。

## 3. 测试结果

```
cargo test --workspace（CARGO_TARGET_DIR 见上，CARGO_INCREMENTAL=0）
  diskwhy-core 单元测试 16 passed
  action_tests          26 passed
  dedup_tests            5 passed, 1 ignored
  explain_tests         30 passed   ← 新增 6 个：every_review_rule_has_lose_kind（75 条 review 规则都有后果类型 + README 例子抽查）、
                                       lose_kind_validation（review 缺字段 / 不认识的值报错、顺序）、explanations_carry_lose_kind
                                       （JSON 是 "lose_kind":"irreversible"、未知 / AI → unsure）、heuristic_review_results_always_have_lose_kind、
                                       cleanup_candidates_group_by_rule_and_respect_guards、cleanup_candidate_with_stricter_rule_inside_is_not_deleted_whole
  mock_tests             8 passed
  real_fs_tests          6 passed
  合计 91 passed, 0 failed, 1 ignored

npm test（vitest 5.0.1）
  Test Files  3 passed (3)      capacity.test.ts / labels.test.ts / treemap.test.ts
  Tests      24 passed (24)     新增：容量柱段高（最小段高、合计、零段）、标签防重叠（推开、贴底、放不下时铺满、引线）、
                                数字格式；lose_kind → 中文标签八种 + 只有“找不回来”实心 + 取最坏；分类归并；路径缩写；外壳文件夹合并

npm run build（vue-tsc --noEmit && vite build）  ✓ built，index.js 131 kB（gzip 48.6 kB）
npm run tauri build -- --no-bundle                Finished `release` profile … Built application at: …\target\release\diskwhy.exe
```

`tauri build` 用了 `--no-bundle`（和阶段 D / E 一样）：这台机器上没有缓存 NSIS，打安装包要让 Tauri CLI 去下载 NSIS 工具链，这次没有下载；产物是 `diskwhy.exe`，已复制到 `D:\desk\claudex\13\DiskWhy-app\`（连同 `diskwhy-cli.exe`）。

**真机删除测试（只动测试时新建的文件）**：在 `%LOCALAPPDATA%\dw-phaseF-demo` 下建 38 个假文件（10,747,904 字节）+ 一份只匹配它们的演示用户规则（一条 safe、一条 review / redownload），`diskwhy.exe --scan <该文件夹>` 启动 → 清理页勾上 review 项 → 确认弹层里“删到哪里 → 其他文件夹…”选 `D:\desk\claudex\13\dw-xvol-test`（盘根 `D:\` 普通用户建不了文件夹，预检正确地给出阻止性提示）→ 勾“我知道这些删了的后果” → 清理：成功 38、跳过 0，源文件夹剩 0 个文件 → 隔离区页“还原”：38 个文件、10,747,904 字节全部回到原处，隔离文件夹清空。之后删掉了演示文件、演示规则、测试文件夹，以及测试产生的 `actions.log`（79 行，副本留在 scratchpad）、`quarantine-roots.json`、`ui.json`。**没有对用户的任何真实文件执行删除 / 移动 / 回收**；C 盘上的 12 张截图只调用了预检 `preview_plan`，没有执行。

## 4. 截图（`docs/screenshots/phaseF-*`）

全部来自最终的 release 版 `diskwhy.exe`，普通权限，打开即自动真实扫描 C:\（最后一次 32 秒；之前几次 41–77 秒），UI Automation 点按钮、切主题（首页底栏色块），`PrintWindow` 截窗口。窗口 1800×1125 物理像素 = **1440×900 逻辑像素**（125% 缩放），和设计稿同尺寸。

| 文件 | 内容 |
|---|---|
| `phaseF-{A,B,C}-home.png` | 首页：可放心清理大数字（A 衬线 + 斜体 GB / B 金色细体 / C 窄粗体 + 青柠色块和小插画）、分类小字、主按钮、C 盘竖向容量柱（每段右边写名字和数字、清理后剩余虚线）、底栏扫描信息 + 主题色块 |
| `phaseF-{A,B,C}-explore.png` | 空间分析 · 并排：C:\ 两层方块图（“Users \ king” 外壳合并、选中块描边、无权限块）、右边 king 的子项列表（色点 / 图标 / 名字 / 灰色类别 / 细条 / 大小） |
| `phaseF-{A,B,C}-cleanup.png` | 清理 · 开发工具：左侧分类文字行、表格式列表、可放心清理默认全选、需你判断每行一个后果标签、node_modules 行内展开（这是什么 / 删了会怎样 / 怎么清 / 3 个位置）、底部通栏 |
| `phaseF-{A,B,C}-confirm.png` | 确认弹层（真实预检结果）：清理 N 项、总数 + 清空回收站后 C 盘剩余、需你判断置顶高亮、按分类一行、跳过 N 个 · 正在使用、回收站空间告急的一行 + 改放 D:\、勾选框（未勾 → 按钮禁用） |
| `phaseF-demo-1-confirm-ack.png` … `phaseF-demo-4-restored.png` | 上面的真机删除测试：勾选后可点 → 清理完成（成功 38）→ 隔离区一批 → 还原后 |

演示的 4 张来自倒数第二个构建（和最终版只差：确认弹层里会多列出“非阻止性”提示的原文；演示时没有这类提示）。

浏览器预览（`npm run dev` + 假后端）用 Edge 无头截图做过同样 12 个状态的对照，布局和设计稿基本逐像素一致；上面的正式截图用的是本机真实数据。

## 5. 与设计稿的差异 / 已知问题

1. **数字和条目是本机真实数据**，不是稿子里的示例（比如可放心清理 36.1 GB、C 盘剩余 5.9 GB、“需你判断”里没有 Maven 仓库 / Rust 编译产物）。本机的 Rust 编译产物在 `AppData\Local\DiskWhy-build` 里，而这个文件夹其实被 Claude 桌面版重定向进了它的商店应用包（`Packages\Claude_…\LocalCache`），候选列表有意不列这种位置，所以清理页没有 “Rust 编译产物” 这一行（启发式本身有测试覆盖）。
2. **截图右边 / 下边有约 10px 的窗口阴影**：窗口去掉了系统边框（自绘标题栏），`PrintWindow` 把 Windows 给无边框窗口加的阴影区域也截了进去。
3. **方块图没有截到悬停卡片**：这台机器上模拟鼠标无效（同阶段 C / E），悬停卡片（名字、大小、这是什么、后果标签）已实现但只在浏览器预览里看过。真实 C 盘里 AppData 占 king 的 85%，所以方块图大部分是灰色的“先别动”，不如稿子里五颜六色。
4. **分类**：稿子左栏有“大文件”，规则库里没有对应分组，实际分类是 系统缓存 / 浏览器 / 开发工具 / AI 模型 / 应用缓存 / 软件数据（按规则文件分组 + 类别归并）。
5. **预检慢**：默认勾选（27 组，含 9.7 GB 的临时文件夹）点“清理”后要约 175 秒才出确认弹层（三次分别 710 / 181 / 174 秒，第一次是因为截图途中窗口被最小化了，恢复后才继续）——后端要逐个文件取指纹、问 Restart Manager 谁在占用（阶段 D 的护栏逻辑，没改）。按钮上会显示“正在检查 N 秒”。
6. **和稿子相比多出来的东西**：回收站模式下总数旁多一行小字“清空回收站后”（回收站在 C 盘，删了不会立刻腾空间，不写会误导）；“删到哪里”下拉里多一个“其他文件夹…”（普通用户在 D:\ 根目录建不了隔离区，预检会拦下）；其余不阻止执行的提示（隔离区和要清理的在同一个盘、有几项合并了）原文列在弹层里。
7. **确认方式变了**：阶段 D 的“输入合计 GB 才能执行”换成了稿子里的勾选框，而且只有清单里有“需你判断”的项时才出现；后端的全部护栏（保护路径、档位、规则一致性、热文件 / 云文件 / 被占用跳过、阻止性警告时不能执行）没有变。
8. **去掉的界面**：阶段 E 的右侧详情面板（六项说明、keep 项“该怎么腾”的命令指引、打开系统设置、文件属性）没有出现在新稿子里，已删掉；“看看都是什么”列表点一行在行内展开 这是什么 / 删了会怎样 / 怎么清（认不出来且开了 AI 时有“问 AI”）。`get_guidance` 命令还在后端，但界面暂时不再调用。快照打开 / 保存、以管理员重启挪到了设置页。
9. 确认弹层标题的“N 项”数的是清理页里的行（组），不是文件数；跳过的数的是文件（“跳过 539 个”）。
10. 只在 125% 缩放、1440×900 下对照过稿子；最小窗口 1100×720 能用（布局是弹性的），但没有逐页截图检查。第一次启动按系统深浅色选默认主题（深 → B）的逻辑写了、在浏览器预览里试过，没有在系统深色模式下真机验证（这台机器是浅色）。
11. `npm run tauri build` 带了 `--no-bundle`，没有生成 NSIS 安装包（原因见第 3 节）。
