# 阶段 H 记录：后果标签改为具体短语 + 建议行 + 扫描进度不再显示超容量数字

日期：2026-09-30　机器：Windows 11 Pro 10.0.26200，C 盘 199 GB（已用 197 GB），125% 缩放，普通用户权限
基于 master `8410380`（阶段 G）。构建目录：`CARGO_TARGET_DIR=C:\Users\king\AppData\Local\DiskWhy-build\target`

## 1. 扫描进度（用户反馈：容量柱满到顶，左边却写“已扫描 230+ GB”）

原因：扫描进度回调里的 `bytes` 是去重前的原始累计（WinSxS 等硬链接、商店应用重定向会重复算），扫完才去重，所以中途可以比整个盘还大。

改法（只改界面，核心没动）：
- 扫描中**不显示任何扫描字节数**。左边大数字换成看过的文件数：「86.5 万个文件」（1 万以下直接写数字），小字是已用时间「正在扫描 17 秒」，下面仍是当前路径和「取消」。
- 右边容量柱扫描中只画盘自己报告的两段：「剩余 2.4 GB」「已用 197 GB」，已用段是流动的“扫描中”样式（不定进度），扫完再换成 剩余 / 可放心清理 / 需你判断 / 其他 四段。
- 纯函数放在 `src/scanview.ts`（`filesCount` / `elapsedText` / `scanSegs`），有单元测试。
- 真机验证：自动扫描 C:\（32 秒左右），UI Automation 每 0.8 秒读一遍窗口里全部文字、找所有“数字 + GB”，记录在 `docs/screenshots/phaseH-scan-texts.txt`：整个扫描过程出现过的最大数字是盘的总容量 **199**（“197 / 199 GB”），其次是已用 197；没有任何数字超过 199 GB。

## 2. 后果标签：从 8 个笼统词换成每条规则自己的具体短语

- 规则新字段 `lose_short`：这一项删了**具体**会怎样，≤ 8 个字（按字符数，空格和字母也算）。`review` 档必填，`safe` / `keep` 可选但写了也不能超长；启动校验（`rules::validate`，常量 `LOSE_SHORT_MAX = 8`）不过的规则不加载，用户自定义规则同样适用。界面从不截断，太长就改规则里的字。
- `lose_kind` 保留：仍决定标签颜色（找不回来 = 实心，其余浅底）、排序和测试。
- `Explanation` / `CandidateGroup` / `TreemapNode` 都带 `lose_short`；清理页行上、展开行「删了会怎样」开头、看看都是什么的展开行、方块图悬停卡片都显示它。没有时退回 `lose_kind` 的通用说法。
- 启发式按名字逐个写（`heuristics::heuristic_lose_short`），“几乎全是某某”的容器跟着认出来的那一层走；完全认不出 = 「不清楚里面是什么」，AI 推测 = 「还没确认是什么」，safe 规则落在个人文件夹里被降成 review = 「在你的文件夹里」。
- 8 个通用兜底标签改成：找不回来 / 以后修不了 / 可能有你的东西 / 删了要重装 / 要重新登录 / 少一个功能 / 要重新下载 / 下次会慢一点。
- C（卡通）主题的标签改用界面字体：标签文字来自规则，打包的手写体子集只含界面固定文案，新短语里有缺字会混排。

### 2.1 全部 75 条 review 规则

| # | 规则 id | 分组 | lose_kind | lose_short（行上的标签） |
|---|---|---|---|---|
| 1 | `huggingface-hub` | ai-models | redownload | **模型要重新下载** |
| 2 | `huggingface-datasets` | ai-models | unsure | **可能混着你的数据** |
| 3 | `ollama-models` | ai-models | redownload | **模型要重新下载** |
| 4 | `ollama-local-data` | ai-models | redownload | **只丢日志和更新包** |
| 5 | `torch-hub` | ai-models | redownload | **模型要重新下载** |
| 6 | `modelscope-cache` | ai-models | redownload | **模型要重新下载** |
| 7 | `insightface-models` | ai-models | irreversible | **模型可能下不回来** |
| 8 | `paddle-models` | ai-models | redownload | **模型要重新下载** |
| 9 | `lm-studio-models` | ai-models | redownload | **模型要重新下载** |
| 10 | `whisper-models` | ai-models | redownload | **模型要重新下载** |
| 11 | `keras-models` | ai-models | unsure | **可能有你的模型** |
| 12 | `nvidia-ngx-models` | ai-models | unsure | **显卡功能可能失效** |
| 13 | `chrome-on-device-model` | browsers | feature | **AI 功能用不了** |
| 14 | `wechat-files-cache` | cn-apps | unsure | **可能碰到聊天记录** |
| 15 | `wechat-appdata` | cn-apps | relogin | **微信要重新登录** |
| 16 | `qq-appdata` | cn-apps | relogin | **QQ 要重新登录** |
| 17 | `wemeet-data` | cn-apps | unsure | **腾讯会议要重登录** |
| 18 | `feishu-larkshell` | cn-apps | relogin | **飞书要重新登录** |
| 19 | `feishu-sdk-storage` | cn-apps | irreversible | **飞书草稿会没了** |
| 20 | `feishu-embedded-browser` | cn-apps | relogin | **飞书网页要重登** |
| 21 | `dingtalk-data` | cn-apps | irreversible | **没上传的文件会丢** |
| 22 | `baidu-netdisk-ui-data` | cn-apps | relogin | **网盘要重新登录** |
| 23 | `thunder-data` | cn-apps | unsure | **下载记录会没了** |
| 24 | `netease-music-data` | cn-apps | relogin | **网易云要重新登录** |
| 25 | `qqmusic-data` | cn-apps | unsure | **QQ音乐要重登录** |
| 26 | `kugou-data` | cn-apps | unsure | **酷狗要重新登录** |
| 27 | `bilibili-data` | cn-apps | relogin | **B站要重新登录** |
| 28 | `wps-roaming` | cn-apps | irreversible | **没同步的文档会丢** |
| 29 | `npm-global` | dev-caches | reinstall | **全局命令要重装** |
| 30 | `uv-root` | dev-caches | unsure | **里面混着别的东西** |
| 31 | `uv-python-tools` | dev-caches | reinstall | **uv 工具要重装** |
| 32 | `cargo-home` | dev-caches | reinstall | **Rust 要重装** |
| 33 | `rustup-toolchains` | dev-caches | reinstall | **Rust 要重装** |
| 34 | `maven-repo` | dev-caches | irreversible | **要重新下载依赖** |
| 35 | `docker-local` | dev-caches | irreversible | **镜像和容器会没了** |
| 36 | `vscode-family-root` | dev-caches | reinstall | **编辑器设置会丢** |
| 37 | `vscode-family-extensions` | dev-caches | reinstall | **编辑器插件要重装** |
| 38 | `jetbrains-local` | dev-caches | unsure | **在用版本设置会丢** |
| 39 | `visual-studio-cache` | dev-caches | slower | **VS 下次开得慢** |
| 40 | `visual-studio-package-cache` | dev-caches | repair | **VS 修复要重下** |
| 41 | `claude-desktop-vm` | misc | irreversible | **工作区数据会丢** |
| 42 | `claude-desktop-data` | misc | relogin | **桌面版要重新登录** |
| 43 | `claude-desktop-package` | misc | unsure | **混着别的程序数据** |
| 44 | `claude-code-data` | misc | irreversible | **历史对话全没了** |
| 45 | `codex-cli-data` | misc | irreversible | **对话和图片会没了** |
| 46 | `dot-cache-root` | misc | unsure | **里面混着别的东西** |
| 47 | `office-file-cache` | misc | irreversible | **没上传的改动会丢** |
| 48 | `outlook-ost` | misc | redownload | **邮件要重新下载** |
| 49 | `nvidia-local` | misc | unsure | **里面混着别的东西** |
| 50 | `nvidia-app-updates` | misc | repair | **重装驱动要重下** |
| 51 | `nvidia-driver-installers` | misc | repair | **重装驱动要重下** |
| 52 | `nvidia-app-data` | misc | reinstall | **显卡软件要重装** |
| 53 | `logitech-ghub` | misc | reinstall | **罗技软件要重装** |
| 54 | `logitech-ghub-cache` | misc | unsure | **罗技组件要重下** |
| 55 | `discord-data` | misc | relogin | **要重新登录** |
| 56 | `vscode-home-dir` | misc | reinstall | **所有插件要重装** |
| 57 | `antigravity-home-dir` | misc | reinstall | **编辑器插件要重装** |
| 58 | `openai-local` | misc | unsure | **里面有在用的程序** |
| 59 | `codex-desktop-local` | misc | unsure | **桌面版可能打不开** |
| 60 | `codex-runtimes-cache` | misc | unsure | **文档类功能用不了** |
| 61 | `codex-runtimes-previous` | misc | repair | **出错时没法回退** |
| 62 | `aclos-game-recorder` | misc | irreversible | **游戏录像会没了** |
| 63 | `aclos-highlight-videos` | misc | irreversible | **精彩录像会没了** |
| 64 | `aclos-old-installer` | misc | repair | **修复时要重下** |
| 65 | `pandoc-local` | misc | reinstall | **文档转换用不了** |
| 66 | `windows-old` | windows-system | repair | **不能退回旧系统** |
| 67 | `hiberfil` | windows-system | feature | **不能休眠** |
| 68 | `recycle-bin` | windows-system | irreversible | **回收站的东西没了** |
| 69 | `windows-logs` | windows-system | repair | **更新出错难排查** |
| 70 | `package-cache` | windows-system | repair | **卸载软件会出错** |
| 71 | `msix-localcache` | windows-system | unsure | **混着别的程序数据** |
| 72 | `store-app-package` | windows-system | irreversible | **应用里的内容会丢** |
| 73 | `root-tmp-dir` | windows-system | unsure | **可能有你放的文件** |
| 74 | `root-temp-dir` | windows-system | unsure | **可能有你放的文件** |
| 75 | `amd-installer-files` | windows-system | repair | **重装驱动要重下** |

### 2.2 启发式 / 兜底

| 来源 | lose_short |
|---|---|
| rust-target、dotnet-build（编译产物） | 下次编译会变慢 |
| steam-cache | 游戏头几次会卡 |
| node-modules | 项目要重装依赖 |
| model-weights、ollama-models-dir | 模型要重新下载 |
| downloaded-installer、conda-pkgs | 安装包要重新下 |
| gradle-cache、gradle-home、flutter-cache、pub-cache、pnpm-store | 要重新下载依赖 |
| steam-workshop | 创意工坊要重下 |
| python-venv、python-env | 项目要重建环境 |
| python-runtime | 解释器要重装 |
| flutter-sdk | SDK 要重装 |
| visual-studio-install | VS 要重装 |
| app-install-elsewhere | 软件要重装 |
| android-sdk、android-sdk-part | 安卓工具要重装 |
| dev-sdk-container | 开发工具要重装 |
| conda-env、conda-base | 环境要重新建 |
| conda-envs、conda-root | 所有环境会没了 |
| driver-packages | 重装驱动要重下 |
| game-highlights | 游戏录像会没了 |
| media-files | 照片视频可能没了 |
| android-avd、android-user-home | 模拟器数据没了 |
| chat-files-cache | 聊天图片会没了 |
| chat-files、chat-files-container | 聊天文件会没了 |
| user-projects | 你的项目会没了 |
| virtual-disk、vm-hyperv、vm-vmware、vm-virtualbox | 虚拟机会没了 |
| chromium-cache | 可能不只是缓存 |
| logs-dumps | 少了排错记录 |
| temp-files | 一般没什么影响 |
| registry-install-dir、registry-program-dir | 软件会打不开 |
| registry-appdata | 某个软件会出问题 |
| game-files、game-tencent(-root)、game-epic、game-minecraft、steam-library / game / root | 游戏要重新下载 |
| dominant-child（几乎全是某某） | 跟着里面那一层 |
| 完全认不出（unknown） | 不清楚里面是什么 |
| AI 推测（llm） | 还没确认是什么 |
| safe 规则落在个人文件夹、被降成 review | 在你的文件夹里 |

## 3. 展开行：这是什么 / 删了会怎样 / 建议

清理页和看看都是什么的展开行都只回答三件事，顺序和标签固定：「这是什么」（规则 `what`）、「删了会怎样」（**lose_short。** + 规则 `lose`）、「建议」（**第一句**由档位 + 怎么清推出来 + 规则 `how_detail` 作第二句）。规则的 `why` 没放进来（三行已经够密，和稿子的行高一致）；多位置的组下面仍列「位置」，认不出的项仍有「线索」「问 AI」。

| 条件（`src/labels.ts` 的 `advice`） | 建议第一句 |
|---|---|
| safe 且 how 含 delete | 可以删，会自动重建 |
| review 且 how 含 delete | 看一眼里面，确定不要了再删 |
| how 含 builtin_tool，规则在 windows-system 组 | 别直接删，用 Windows 自带工具清（能打开系统设置页时后面多一个「打开存储设置」，走阶段 E 的 `get_guidance` / `open_system_settings` 白名单） |
| how 含 builtin_tool，其他组（微信 / 飞书 / conda / VS 安装器…） | 别直接删，用软件自带的清理 |
| how 只有 manual | 自己打开看看，手动删 |
| how 只有 migrate（keep 档有 migrate 也算） | 别删，搬到 D 盘更好 |
| none / keep / 认不出来 / AI 推测 | 别动 |

判断顺序：delete > builtin_tool > manual > migrate > 别动。“Windows 自带工具”只用于 windows-system 组，否则“用 Windows 工具清微信缓存”会说错。为了让第二句不和第一句打架，改了 3 条规则的 `how_detail`（`package-cache`、`nvidia-app-data`、`logitech-ghub` 原来以“不建议手动删”开头）。

## 4. 其他界面文字（一眼看懂）

- 清理页：分组提示「删了会自动重建」「看过再勾」；不能勾的可放心项标签「用自带工具 / 搬到 D 盘 / 要手动删」（原 用工具清 / 可迁移 / 手动清）；底栏「可释放」→「能腾出」。
- 看看都是什么：列表灰色类别「编译产物」→「编译生成」、「虚拟磁盘」→「虚拟机」；合并的小文件「零散 N 项」→「小文件 N 个」。
- 设置：「AI 推测」→「AI 猜文件夹」，「Key」→「密钥」，「数据」→「其他」，「规则」→「识别规则」，「快照」→「扫描存档」；主题名「趣味」→「卡通」（和用户的叫法一致）。
- 首页 / 容量柱的“可放心清理 / 需你判断 / 其他 / 清理后剩余”、左栏分类名没改（已经是一眼能懂的词）；确认弹层里需你判断项的标签也改成了 lose_short（见第 8 节）。

## 5. 测试

```
cargo test --workspace（CARGO_INCREMENTAL=0）
  diskwhy-core 单元 16 · action_tests 29 · dedup_tests 5（1 ignored）· explain_tests 34 · mock_tests 8 · real_fs_tests 6
  合计 98 passed, 0 failed, 1 ignored
  新增：every_review_rule_has_short_concrete_tag（75 条都有、≤ 8 字、用户给的 13 个例子逐条核对）、lose_short_validation
  （缺 / 空 / 9 个字报错，“Rust 要重装”8 个字通过，safe 写超长也报错）、explanations_and_candidates_carry_lose_short、
  heuristic_review_results_have_short_tags
npm test：4 个文件 31 passed（新增 scanview.test.ts；labels.test.ts 加了兜底词、loseTag、advice）
npm run build ✓；npm run tauri build -- --no-bundle ✓（diskwhy.exe）；cargo build --release -p diskwhy-cli ✓
```

exe 已复制到 `D:\desk\claudex\13\DiskWhy-app\`（DiskWhy.exe、diskwhy-cli.exe，12:48；之前那份是阶段 G 12:16 的，这次的构建基于包含阶段 G 的 master）。

## 6. 截图（`docs/screenshots/phaseH-*`，最终 exe，卡通主题，真实自动扫描 C:\，UI Automation 点按钮，PrintWindow 截图）

| 文件 | 内容 |
|---|---|
| `phaseH-1-scanning-{1,2,3}.png` | 扫描中：文件数大数字（随扫描增长，单位“万个文件”）、已用秒数、剩余 / 已用两段容量柱；没有扫描字节数 |
| `phaseH-scan-texts.txt` | 扫描全过程每 0.8 秒一行的界面文字（最大数字 199 = 盘总容量） |
| `phaseH-2-home.png` | 扫完的首页 |
| `phaseH-3-cleanup-package-cache.png` | 系统缓存：需你判断各行的新标签；「安装包副本」展开（卸载软件会出错 / 自己打开看看，手动删） |
| `phaseH-4-cleanup-recycle-bin.png` | 「回收站」展开：回收站的东西没了（实心）/ 用 Windows 自带工具清 + 打开存储设置 |
| `phaseH-5-cleanup-tmp.png` | 「C:\tmp」展开：可能有你放的文件 |
| `phaseH-6-cleanup-wechat.png` | 软件数据：「微信程序数据」展开（微信要重新登录） |
| `phaseH-7-cleanup-feishu.png` | 「飞书本地数据库」展开：飞书草稿会没了 / 用软件自带的清理 + 4 个位置 |
| `phaseH-8-explore-claude.png`、`phaseH-9-explore-insightface.png` | 看看都是什么 · 列表：展开行 这是什么 / 删了会怎样（标签）/ 建议 |

截图过程中有一次 UI Automation 误点到清理页底栏的「清理」按钮，打开了确认弹层（只做了只读的预检），随后点「取消」关掉；**没有执行任何删除 / 移动 / 回收**。`%LOCALAPPDATA%\DiskWhy\ui.json` 本来就是 `C`，没改。

## 7. 没做 / 已知问题

- 只在卡通主题下截图；浅色 / 深色共用同一套布局和组件，没有逐张截图。
- 首页扫描中是“不定进度”（不给百分比）：扫描前不知道总文件数，去重前的字节又不可信，宁可不给。

## 8. 补：确认弹层也用具体后果短语

`ConfirmDialog.vue` 需你判断的项按候选组取 `loseTag`（lose_short，没有再退回通用词），颜色仍按 lose_kind（找不回来实心）。卡通主题下标签走 base.css 的全局 `.tag`，已经是界面字体。截图 `phaseH-10-confirm.png`：勾上「AMD 驱动安装包」后点清理，弹层里显示「重装驱动要重下」；只做了只读的预检，随后点 × 关掉，没有执行。npm test 31 passed，cargo test 98 passed / 1 ignored；exe 重新构建并复制到 `D:\desk\claudex\13\DiskWhy-app\`。
