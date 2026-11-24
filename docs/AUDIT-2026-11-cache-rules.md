# 缓存规则复查（2026-11-02，本机实测）

这次用 Mole 那三个问题，把 DiskWhy 里所有“看着像缓存”的规则重新过一遍：

1. **谁写的？**
2. **重启以后谁还会再读它？**
3. **要是删错了，它怎么回来？**

三个问题有一个答不上，就不让它当“可放心清理”（safe）。

另外按 Mole 的第二条把可删的东西分两类：
- **编译产物**：花 CPU 时间就能重新生成，删了只是变慢。
- **下载缓存**：要联网才能回来。可以留在 safe，但要写清楚“要重新下载”，不能只说“变慢”。

只读审计：没改任何规则文件和源码，也没碰 `%LOCALAPPDATA%\DiskWhy`。脚本和原始结果在
`C:\Users\king\AppData\Local\Temp\claude\D--desk-claudex-13\13d5dbc4-6fbe-45e4-b54b-030e4a6a3581\scratchpad\cache-audit\`。

---

## 一、怎么查的

- **规则怎么挑出来**：216 条规则里挑出 90 条要审，标准是满足下面任意一条：
  - tier = safe；
  - 模式里有 `*` 或 `**`；
  - id 或路径里含 cache / temp / crash / log / update / installer / download。
- **匹配脚本**：自己写了一份只读的匹配脚本 `matcher.py`，照搬三个文件的语义：
  - `pattern.rs`：段匹配、`*` / `?` / `**`、大小写不敏感、按具体程度挑最具体的；
  - `env.rs`：变量白名单、去掉结尾反斜杠；
  - `rules.rs`：kind / when_child、self 和 tree 的继承。
- **when_child 按 DiskWhy 实际看到的算**：扫描时每个文件夹只保留最大的 50 个文件，其余的并成“其余小文件”节点。脚本按这个口径判断 when_child，同时也记下“文件系统里实际有”的结果。实测两种口径没有差别。
- **没有重定向 LOCALAPPDATA，也没跑 DiskWhy。**
- **重要：要看“物理视图”**
  - 这次审计的进程跑在 Claude 商店版的应用包里，看到的 AppData 是“合并视图”：商店应用写进 `Packages\Claude_…\LocalCache` 的文件，也会显示在真实的 AppData 位置。
  - 所以第二遍改从 `\\localhost\C$` 读。这是不带重定向的物理视图，和用户直接运行 DiskWhy 时看到的一样。
  - 例子：`%LOCALAPPDATA%\pip\cache` 在合并视图里有 1.77 GB，物理上只有 64.5 MB。另外那 1.7 GB 其实在 Claude 应用包的 LocalCache 里，DiskWhy 的清理页本来就不列这一块。
- **大小口径**：下面的大小都是文件逻辑大小之和，不是实际占用的簇。扫的是不提权的 C 盘。有 59 个目录读不了，比如 Prefetch、`System32\config`、`ServiceProfiles\NetworkService`。
- **删除时的保护（看了源码）**：
  - 30 分钟内改过的文件会跳过（`HOT_WINDOW_SECS`）；
  - 被占用的文件会跳过（用 Restart Manager 查）；
  - 但**超过 30 分钟、当前没被打开的文件一律照删**。

---

## 二、逐条结论

“命中”指本机物理视图里这条规则作为最具体规则精确命中的路径。商店应用重定向区（`Packages\*\LocalCache\{Local,Roaming,LocalLow}\…`）不上清理页，单独标注。

### 2.1 safe 档

| id | 档 | 本机命中 | 谁写 | 重启后谁读 | 删错了怎么回来 | 结论 |
|---|---|---|---|---|---|---|
| windows-temp | safe | 1 处 / 48.4 MB。全部不到 7 天，`vminst.log` 正在写 | 系统服务、安装程序 | 等重启的安装操作可能还要用 | 回不来。Windows 自带的“磁盘清理”只删 7 天没碰过的 | **收紧**：how 改成只用系统工具；引擎支持 min_age_days 后再恢复直接删 |
| user-temp | safe | 1 处 / 3.95 GB。**全部不到 7 天**：VS 安装器的包 3.3 GB、VS Code 下好待装的更新 240 MB、Claude Code 正在用的 `claude\` 12 MB（105 个文件，几分钟前还在写）、UU 加速器的 `UU1175.tmp`（已经登记在 PendingFileRenameOperations 里等重启处理） | 各种程序 | VS Code 下次启动读 `update-metadata.json` 装更新；VS 安装器续装；正在跑的会话；等重启的操作 | 大多要联网重下（VS 包 3.3 GB），会话里的中间文件回不来 | **收紧**（同上） |
| windows-update-download | safe | 1 处 / 3.4 MB | Windows 更新服务 | 更新服务下次扫描 | 联网重下 | OK，补 lose_kind = redownload |
| delivery-optimization | safe | 0（不提权读不了） | 传递优化服务 | 服务自己 | 联网重下 | OK，补 redownload |
| thumbnail-cache | safe | 31 项 / 66.4 MB：文件夹本身 + 30 个 db。这个文件夹里还有 `ExplorerStartupLog_RunOnce.etl`、`RecommendationsFilterList.json` | explorer.exe | explorer 下次显示缩略图和图标 | 从原文件重新生成（耗 CPU） | **收紧**：去掉“整个文件夹”这条模式，只认 db 文件 |
| inetcache | safe | 1 处 / 0 MB，只有 `IE\` 和 `Content.IE5\` | 旧 IE 内核的网页控件（WinINet） | WinINet 查缓存时 | 联网重下。但 Outlook 打开附件时的副本 `Content.Outlook` 也放在这个文件夹下面 | **收紧**：只认 IE、Content.IE5 |
| directx-shader-cache | safe | 1 处 / 1.5 MB | Direct3D | 下次编译同一个着色器 | 重新编译。“磁盘清理”自带这一项 | OK，补 slower |
| windows-error-reports | safe | 2 处 / 44.9 MB：`CrashDumps` 里 3 个 dmp（wallpaper64、FileCoAuth），加上 WER 的 ReportArchive | WER | 只在上传报告时读 | 不需要回来（日志） | OK |
| memory-dumps | safe | 2 处 / 0 MB | 内核蓝屏转储 | 没人读（排查用） | 不需要 | OK |
| store-app-temp | safe | 362 处 / 3.8 MB | 商店应用 | 不保证有人读。微软原话：临时数据存储“could be removed at any time” | 不需要 | OK |
| chrome-cache | safe | 52 处 / 694 MB。其中 **Service Worker 部分 11 处 / 57.5 MB**：Gmail 离线外壳 `sw-/mail/u/0/`、MSN、atomgit 的 `pwa-offline-v1` | Chrome | Chrome 下次打开网页 | Cache、GPU 这些：联网重下或重新编译。CacheStorage：网站自己存的数据，不保证回得来 | **拆分**：把 Service Worker 两条移出去 |
| edge-cache | safe | 10 处 / 258.5 MB（Service Worker 部分 2 处 / 47.8 MB） | Edge | 同上 | 同上 | **拆分**（同上） |
| chromium-browsers-cache | safe | 0 | Brave、Vivaldi、Opera | 同上 | 同上 | **拆分**（同上） |
| firefox-cache | safe | 0（没装） | Firefox | Firefox | cache2 联网重下，startupCache 重新生成 | OK，补 redownload（没实测） |
| chromium-generic-cache | safe | 34 处 / 690.6 MB（另有 9 处在重定向区）。**每一处都有 Cache_Data 或 data_0/1**。同时挡住了 229 个同名但不是 Chromium 的文件夹：`claude-cli-nodejs\Cache`、`checkpoint-nodejs\Cache`、conda 的 Cache、`fontconfig\cache`、`Shared Dictionary\cache`、`Doubao\User Data\repair\cache`（里面是修复程序 exe）等 | Chromium 网络模块 | 下次请求同一个资源 | 联网重下 | OK，补 redownload |
| chromium-generic-code-cache | safe | 34 处 / 74.4 MB（另有 9 处在重定向区）。没命中的 71 个几乎都是空文件夹 | V8 引擎 | 下次加载同一段脚本 | 重新编译 | OK，补 slower |
| chromium-generic-gpu-cache | safe | 94 处 / 127.1 MB（另有 36 处在重定向区）。没命中的 212 个大多是空文件夹 | GPU 进程、Dawn | 下次画界面 | 重新编译着色器 | OK，补 slower |
| **chromium-generic-sw-cache** | safe | 29 处 / 167.5 MB：**Razer Synapse 141.7 MB，里面有一个名叫 `DeviceDataJson` 的 cache**；NVIDIA App 的 `nvidiaapplication-offline-v1` 和 `allAppsMetaData`；ACLOS 的 workbox 预缓存；Windows CBS 的 `resourceCache`；微信的 `offline_page` | 各网站、网页应用自己的脚本 | 下次打开这个网页应用（断网时全靠它） | 只能联网让网站重新拉；网站往里存的不是下载来的内容就回不来。Chromium 把它算作“网站数据”（和 IndexedDB 同类），不算“缓存” | **降档 review** |
| crashpad-reports | safe | 37 处 / 0 MB（另有 6 处在重定向区），其中 15 个是空文件夹。其余都是标准的 Crashpad 记录库：settings.dat、metadata、reports、attachments | crashpad_handler | 程序启动时打开记录库（没有就新建），读 settings.dat 里的客户端编号和上传开关 | 自动新建空库；上传开关由程序按自己的设置重写；没传出去的报告丢掉（本来就该丢） | **收紧**：加 when_child，去掉没人认领的 `Crash Reports` |
| npm-cache | safe | 1 处 / 1.76 GB：`_cacache` 1.69 GB、`_npx` 106 MB | npm、npx | 下次 npm install；npx 启动工具，包括用 npx 启动的 MCP 服务 | 联网重下；断网时 npx 工具起不来 | OK，改文案 + redownload |
| pnpm-store | safe | 0 | pnpm | pnpm install（项目里的 node_modules 是硬链接） | 联网重下 | OK，补 redownload |
| yarn-cache | safe | 1 处 / 0 MB（`Yarn\Cache\v6` 是空的） | Yarn 1 / Yarn Berry | Yarn 1 只在安装时读；**Yarn Berry 全局缓存在 PnP 项目运行时被直接读 zip** | Berry：每个 PnP 项目都要重新 `yarn install`（联网），重装之前项目跑不起来 | **拆分**：Berry 单独一条 review |
| bun-cache | safe | 0 | bun | bun install | 联网重下 | OK，补 redownload |
| playwright-browsers | safe | 1 处 / 0 MB | playwright install | 跑自动化时 | 联网重下几百 MB | OK，补 redownload |
| electron-download-cache | safe | 0 | electron、electron-builder、node-gyp | 构建时 | 联网重下 | OK，补 redownload |
| pip-cache | safe | 1 处 / 64.5 MB（http-v2） | pip | pip install | 联网重下 | OK，补 redownload |
| uv-cache | safe | 1 处 / 392.7 MB（archive-v0 375 MB） | uv | uv 每次装包；uvx 的临时环境（environments-v2） | 联网重下。**uv 官方原话：直接改缓存“never safe”** | **收紧**：how 只留 uv 自己的命令 |
| conda-pkgs | safe | 0（conda 装在 D 盘） | conda | conda install | 联网重下 | OK，补 redownload |
| cargo-registry | safe | 1 处 / 0 MB | cargo | cargo build 直接读 `registry\src`、`git\checkouts` | 联网重下；离线编译不了 | OK，改文案 + redownload |
| go-cache | safe | 0 | go | go build | go-build：重新编译；pkg\mod：联网重下 | OK，改文案 + redownload |
| gradle-cache | safe | 0 | Gradle | 构建时 | 联网重下 | OK，改文案 + redownload |
| nuget-cache | safe | 0 | NuGet | **用 PackageReference 的项目编译时直接用 `.nuget\packages`** | 联网 `dotnet restore`；VS 可能要重新加载解决方案 | OK，改文案 + redownload |
| docker-logs | safe | 1 处 / 0.1 MB | Docker Desktop | 没人读 | 不需要 | OK |
| vscode-family-cached | safe | 10 处 / 753 MB，其中 `CachedExtensionVSIXs` 724 MB 是下载来的扩展包 | VS Code 系编辑器 | CachedData 启动时读；VSIX 重装扩展时读 | CachedData 重新编译；VSIX 联网重下 | OK，补 redownload |
| jetbrains-caches | safe | 0（没装） | JetBrains IDE | 打开项目时 | 重建索引。LocalHistory 不在这四个子文件夹里，由 jetbrains-local（review、手动）兜着 | OK，补 slower（没实测） |
| ollama-updates | safe | 1 处 / 1.46 GB | Ollama 更新程序 | Ollama 下次重启时装更新 | 联网重下 | OK，补 redownload |
| **wechat-render-cache** | safe | 2 处 / 441.3 MB。`xwechat\radium` 里**不只是缓存**：`web\profiles\multitab_*` 是完整的浏览器资料（Cookie、History、Local Storage、IndexedDB、Web Data）；`users\<wxid>\applet\local\<小程序>\usr` 和 `usrmmkvstorage*` 是小程序的本地存储；还有 `mmkv\`、`device_uuid_0` | 微信 4.x 的 radium | 微信启动时、打开小程序和内置网页时 | 内置浏览器的登录状态、小程序存在本机的数据、设备编号都回不来 | **拆分**：radium 改成 review 且只能用微信自带清理；`xplayer\dump` 并进日志 |
| wechat-logs | safe | 2 处 / 881.9 MB（xlog 日志；crashinfo 是 Crashpad 库） | 微信 | 没人读（反馈问题时上传） | 不需要 | OK，再加上 `xplayer\dump` |
| wechat-update | safe | 1 处 / 0 MB | 微信 | 更新时 | 联网重下 | OK，补 redownload |
| tencent-logs | safe | 1 处 / 0 MB | 腾讯组件 | 没人读 | 不需要 | OK |
| feishu-update | safe | 1 处 / 0 MB | 飞书 | 更新时 | 联网重下 | OK，补 redownload |
| feishu-logs | safe | 1 处 / 0 MB | 飞书 | 没人读 | 不需要 | OK |
| netease-music-cache | safe | 2 处 / 2.01 GB：387 首边听边存的加密歌曲 `.uc` | 网易云音乐 | 再播同一首时 | 联网重新缓冲（会员歌要有会员） | OK，补 redownload |
| **netease-music-logs** | safe | 3 处 / 7.3 MB。**`dumps\` 里不是崩溃转储**，而是今早启动时写的 `cookie_json`（3 KB）和 `appParam` | 网易云音乐 | dumps 那两个文件：不知道谁读 | 不知道 | **收紧**：去掉 dumps |
| gpu-shader-cache | safe | 6 处 / 835 MB（NVIDIA DXCache 824 MB，`.nvph` 文件） | 显卡驱动 | 游戏或程序下次编译着色器 | 重新编译（游戏头一次会卡） | OK，补 slower |
| steam-caches | safe | 0 | Steam | Steam、游戏 | shadercache 重新编译；appcache、depotcache 联网重下 | OK，补 redownload（没实测） |
| raspberry-pi-imager-cache | safe | 1 处 / 0 MB | 树莓派镜像烧录工具 | 烧录时 | 联网重下（GB 级） | OK，补 redownload |
| adobe-media-cache | safe | 0 | Premiere 等 | 打开工程时 | 重新生成（可能很久） | OK，补 slower（没实测） |
| squirrel-temp | safe | 0 | Squirrel 安装器 | 安装进行中才读 | 装到一半被删就重装 | OK（没实测；以后适合加 min_age_days） |
| electron-updater-cache | safe | 2 处 / 321.9 MB：CherryStudio、Typeless 的 `pending\` 安装包。另外 10 个 `*-updater` 是空文件夹，没命中 | electron-updater | 下次检查更新、或退出时安装 | 联网重下 | OK，补 redownload |

### 2.2 只因关键字或通配被列进来的 review / keep 规则

这一批三个问题都答得上，或者本来就不能直接删，**都不用改**（只有 outlook-ost 建议改标签）。

| id | 档 | 本机命中 | 三问简答 | 结论 |
|---|---|---|---|---|
| windows-installer-cache | keep | 1 处 / 3.85 GB，含 `$PatchCache$` 33 MB | Windows Installer 写；卸载、修复、打补丁时读；删了回不来 | OK（`$PatchCache$` 继承 keep，陷阱已挡住） |
| windows-logs | review | 63.2 MB | CBS、DISM 写；排查时读 | OK |
| package-cache | review | 841.8 MB | Burn 安装器写；卸载、修复时读；要重下安装包 | OK |
| msix-localcache / store-app-package | review | 27.9 GB（self）/ 1.08 GB | 商店应用写、自己读 | OK |
| root-tmp-dir / root-temp-dir / amd-installer-files | review | 229.8 MB / 2.2 MB / 24.9 MB | 可能有你放的东西 | OK |
| edge-program / wsl-distro-vhdx | keep | 3.32 GB / 10.63 GB | 程序本体 / 虚拟磁盘 | OK |
| visual-studio-cache / visual-studio-package-cache | review | 44.3 MB / 17.2 MB | VS 写；修复时要用 | OK |
| huggingface-hub、huggingface-datasets、torch-hub、modelscope-cache、lm-studio-models、whisper-models | review | 3.93 GB / 0 / 0 / 0 / 0 / 0 | 模型下载很大、可能要登录或付费 | OK（模型本来就不该是 safe） |
| wechat-files-cache / feishu-sdk-storage / feishu-embedded-browser | review | 0 / 843.4 MB / 44.2 MB | 碰到聊天记录、草稿、登录状态 | OK |
| baidu-netdisk-downloads、netease-music-downloads、qqmusic-downloads、onedrive-folder、outlook-pst、claude-cowork-sessiondata | keep | — | 用户的文件 | OK |
| claude-desktop-vm、claude-desktop-package、dot-cache-root、codex-runtimes-cache、codex-runtimes-previous、aclos-old-installer | review | 0（另有 1 处在重定向区）/ 13.84 GB / 4.12 GB / 2.16 GB / 667.6 MB / 466.8 MB | 已有说明 | OK |
| office-file-cache | review、找不回来 | 20.2 MB / 1199 个文件 | Office 上传中心写；Office 启动时读；**没上传的改动删了就回不来** | OK（陷阱已挡住） |
| outlook-ost | review、要重下载 | 0（没装经典 Outlook） | Outlook 写、自己读；服务器上的能重下，**还没同步的草稿会丢** | **改标签**：lose_kind 按最坏情况改成 irreversible（规则自己的 lose 已经写了“会丢”） |
| nvidia-app-updates / nvidia-driver-installers / logitech-ghub* | review、keep | 3.95 GB / 3.95 GB / 1.40 GB… | 已有说明 | OK |

---

## 三、找到的陷阱

### 3.1 现在就会被预选删掉的（要改规则）

1. **Service Worker 的 CacheStorage 和 ScriptCache 不是缓存，是网站数据**
   - Chromium 源码 `browsing_data_remover.h` 把 `DATA_TYPE_CACHE_STORAGE` 算进 `DATA_TYPE_DOM_STORAGE`，和 IndexedDB、Local Storage 同一类；HTTP 缓存是另一个单独的位 `DATA_TYPE_CACHE`。
   - MDN 也说 Cache API 和 IndexedDB 用同一套配额和清除规则。
   - Chrome 里勾“缓存的图片和文件”**不会**清它。现在 chrome-cache 的 how_detail 说两者等价，是错的。
   - 本机实测：Razer Synapse 往里存了 `DeviceDataJson`（设备数据，不是下载来的），NVIDIA App 存了离线数据，Gmail 存了离线外壳。

2. **微信 `xwechat\radium` 整个被当成“渲染缓存”**
   - 实际里面有：内置浏览器的完整资料（Cookie、历史、Local Storage、IndexedDB、Web Data）、小程序本地存储（`applet\local\<小程序>\usr`、`usrmmkvstorage0/1`）、MMKV 配置、设备编号 `device_uuid_0`。
   - 这是典型的 Mole 式陷阱：名字和位置都像缓存，里面却是状态。

3. **%TEMP% 里的东西都在 7 天以内，而且有正在用的**
   - 4 GB 全部是最近 7 天写的。
   - `claude\` 是 Claude Code 当前会话的临时目录。
   - VS Code 下好、等下次启动安装的更新（`update-metadata.json` + `CodeSetup-*.exe`）。
   - VS 安装器 3.3 GB 的包，删了要重下。
   - 注册表 PendingFileRenameOperations 里登记着 `%TEMP%\UU1175.tmp`，要等重启处理。
   - Windows 自己的磁盘清理默认只删 7 天没访问过的临时文件；DiskWhy 只躲开 30 分钟内改过的和被占用的，超过 30 分钟的照删。

4. **网易云 `dumps\` 里不是崩溃转储，是今早启动时写的 `cookie_json` 和 `appParam`**
   - 谁会再读它不清楚，所以不能放在 safe 的“日志”里。

5. **uv 缓存**
   - uv 官方原话：直接改缓存“never safe”，删文件也算。
   - DiskWhy 直接删时，如果正好有 uv 在跑，被占用的文件会被跳过，其余照删，缓存就坏了一半。

6. **Yarn Berry 全局缓存**（本机没有，按官方文档判断）
   - `enableGlobalCache` 默认开着，PnP 项目运行时直接从缓存里的 zip 读包。
   - 删了不是“下次装包慢”，而是所有 PnP 项目都跑不起来，要逐个重新 `yarn install`。

7. **缩略图规则把整个 `Explorer` 文件夹当成一个候选**
   - 文件夹里还有 `RecommendationsFilterList.json`（开始菜单推荐过滤表）和启动日志，三个问题答不上。

8. **INetCache 文件夹下有 `Content.Outlook`**
   - 经典 Outlook 打开附件时的副本放在这里，改了没另存的话改动只在这里。
   - 本机没装经典 Outlook，这是已知行为，没抓官方文档。

### 3.2 规则现在没碰到、但查过确认安全的

- **Office 上传缓存 `OfficeFileCache`**（20.2 MB）：office-file-cache 是 review、找不回来。没问题。
- **`%LOCALAPPDATA%\Microsoft\Windows\WebCache`**（50 MB）：ESE 数据库，含历史记录索引，正在被占用。没有规则命中（名字是 WebCache，不叫 Cache），结果是“未知”，不会被删。可以考虑加一条 keep 说明。
- **`C:\Windows\Installer\$PatchCache$`**（33 MB）：继承 windows-installer-cache，是 keep。
- **IndexedDB、Local Storage、File System、blob_storage、VS Code 的 `User\History` 和 `Backups`、聊天数据库**：扫过一遍，没有任何 safe 规则落在这些文件夹里面。
- **VS Code 系编辑器**：
  - Code、Trae、Antigravity 的 `User` 是 keep，`Backups` 由 vscode-family-root（review、手动）兜着。
  - **Kiro 和 Qoder 没有规则**，它们的 `User\History`、`Backups` 现在是“未知”，不会被删；但也没有 keep 说明。建议把这两个加进 keep 和 review 列表。
  - 本机 Kiro 的 Backups 里有 1 项未保存的备份。
- **JetBrains LocalHistory**：本机没装。规则只命中 caches、index、log、tmp 四个子文件夹，LocalHistory 落在 jetbrains-local（review、手动）里。
- **`.nuget\packages` 和 `NuGet\v3-cache`**：本机都没有。按微软文档，前者是项目编译时直接用的包，后者是 30 分钟就过期的 HTTP 缓存，都能联网重新下回来，所以留在 safe，但要把“离线编译不了”写清楚。
- **Outlook `.ost`**：本机没有。规则是 review。
- **FontCache**：在 `ServiceProfiles\LocalService` 下，继承 keep。另外 `%LOCALAPPDATA%\Microsoft\Windows\Fonts` 里是**用户自己装的字体**（2.9 MB），没有规则命中，是“未知”，不会被删。
- **thumbcache、D3DSCache、NVIDIA DXCache 和 GLCache**：都属于编译产物，能重新生成，留在 safe。
- **QQ 和微信的聊天数据**（`Documents\Tencent Files`、`xwechat_files`）：没有任何 safe 命中；而且它们在“文档”文件夹下，safe 规则到这里也会被自动降成 review。
- **名字叫 `*.cache` 或 `Caches` 的文件夹**：AppData 里只找到 `%LOCALAPPDATA%\Microsoft\Windows\Caches`（0.7 MB），没有规则命中。

### 3.3 `**` 通配规则的防误伤条件实测

- **Cache**（Cache_Data / data_0 / data_1）：挡住了 229 个同名但不是 Chromium 的文件夹，没有一个漏进来。
- **Code Cache**（js / wasm）、**GPU 和着色器**（index / data_0）：没命中的基本都是空文件夹，挡对了。
- **Crashpad**：没有任何条件。本机命中的都是真的 Crashpad 库，但条件为零，任何叫 Crashpad 的文件夹都会被当成 safe，所以要加条件。
- **Service Worker**：也没有条件。见 3.1 第 1 条。

---

## 四、要改的地方

下面每一块都是**完整替换**或**新增**的 `[[rule]]`，直接粘贴就能用。
- lose_short 的字数都在 8 个以内。
- `verified_on` 统一写 2026-11-02。

### 4.1 browsers.toml

**chrome-cache（替换）**：去掉两条 Service Worker 模式，改 how_detail，补 lose_kind。

```toml
[[rule]]
id = "chrome-cache"
owner = "Google Chrome"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = [
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\Cache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\Code Cache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\GPUCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\DawnCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\DawnGraphiteCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\*\DawnWebGPUCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\ShaderCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\GrShaderCache',
  '%LOCALAPPDATA%\Google\Chrome\User Data\GraphiteDawnCache',
]
what = "Chrome 的网页缓存（图片、脚本、显卡着色器等）"
why = "打开过的网页先存一份，下次打开更快"
lose = "不丢书签、密码和历史；网页第一次打开要联网重新下载，会慢一点。网站存在 Service Worker 里的离线数据不在这一项（单独列在“需要你判断”里）。"
how = ["delete", "builtin_tool"]
how_detail = "关掉 Chrome 后删除；或在 Chrome 的“删除浏览数据”里只勾“缓存的图片和文件”。"
refs = ["https://support.google.com/chrome/answer/2392709"]
verified_on = "2026-11-02"
```

**edge-cache（替换）**

```toml
[[rule]]
id = "edge-cache"
owner = "Microsoft Edge"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = [
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\Cache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\Code Cache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\GPUCache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\DawnCache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\DawnGraphiteCache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\*\DawnWebGPUCache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\ShaderCache',
  '%LOCALAPPDATA%\Microsoft\Edge\User Data\GrShaderCache',
]
what = "Edge 的网页缓存"
why = "打开过的网页先存一份，下次打开更快"
lose = "不丢收藏夹、密码和历史；网页第一次打开要联网重新下载，会慢一点。网站存在 Service Worker 里的离线数据不在这一项。"
how = ["delete", "builtin_tool"]
how_detail = "关掉 Edge（包括后台运行）后删除；或用 Edge 的“清除浏览数据”只勾“缓存的图像和文件”。"
refs = []
verified_on = "2026-11-02"
```

**chromium-browsers-cache（替换）**

```toml
[[rule]]
id = "chromium-browsers-cache"
owner = "Brave / Vivaldi / Opera"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = [
  '%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data\*\Cache',
  '%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data\*\Code Cache',
  '%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data\*\GPUCache',
  '%LOCALAPPDATA%\Vivaldi\User Data\*\Cache',
  '%LOCALAPPDATA%\Vivaldi\User Data\*\Code Cache',
  '%LOCALAPPDATA%\Vivaldi\User Data\*\GPUCache',
  '%LOCALAPPDATA%\Opera Software\Opera Stable\Cache',
  '%LOCALAPPDATA%\Opera Software\Opera GX Stable\Cache',
]
what = "浏览器的网页缓存"
why = "打开过的网页先存一份，下次打开更快"
lose = "不丢书签和密码；网页第一次打开要联网重新下载，会慢一点。"
how = "delete"
how_detail = "关掉浏览器后删除，或用浏览器自带的“清除浏览数据”。"
refs = []
verified_on = "2026-11-02"
```

**chromium-generic-sw-cache（替换，降成 review）**：从上面三条移出去的浏览器 Service Worker 路径，也由这条通配规则接住。

```toml
[[rule]]
id = "chromium-generic-sw-cache"
owner = "Chromium / Electron 程序"
category = "cache"
tier = "review"
lose_kind = "unsure"
lose_short = "离线数据可能丢"
owner_from_path = true
kind = "dir"
match = [
  '%APPDATA%\**\Service Worker\CacheStorage', '%LOCALAPPDATA%\**\Service Worker\CacheStorage',
  '%APPDATA%\**\Service Worker\ScriptCache', '%LOCALAPPDATA%\**\Service Worker\ScriptCache',
]
what = "网页应用的离线存储（Service Worker 的 Cache Storage 和脚本）"
why = "网页和 Electron 程序里的网页（Gmail、飞书文档、Razer、NVIDIA App 等）把页面和数据存在本机，下次秒开、断网也能用。Chromium 把它算作“网站数据”（和 IndexedDB 同类），不算“缓存的图片和文件”"
lose = "大多是能重新下载的页面资源；但网站可以往里存任何东西（本机实测：Razer 存了 DeviceDataJson，NVIDIA App 存了离线数据）。删了这些网页应用要联网重新加载，存在里面、还没传到服务器的内容会丢；只删脚本不删注册信息时，对应网页的离线功能可能要联网再打开一次才恢复。"
how = ["delete", "builtin_tool"]
how_detail = "浏览器里按网站清：设置 > 隐私 > “Cookie 和网站数据”，只清你认得的网站；Electron 程序先完全退出再删。"
refs = [
  "https://chromium.googlesource.com/chromium/src/+/HEAD/content/public/browser/browsing_data_remover.h",
  "https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria",
]
verified_on = "2026-11-02"
```

**crashpad-reports（替换）**：加 when_child，去掉 `Crash Reports`。Windows 上没有已知的程序往 `%LOCALAPPDATA%\…\Crash Reports` 写，本机也是 0 处。

```toml
[[rule]]
id = "crashpad-reports"
owner = "Chromium / Electron 程序"
category = "log"
tier = "safe"
owner_from_path = true
kind = "dir"
match = ['%APPDATA%\**\Crashpad', '%LOCALAPPDATA%\**\Crashpad']
when_child = ["settings.dat", "metadata", "reports"]
what = "程序崩溃报告（Crashpad 记录库）"
why = "Chromium / Electron 程序崩溃时，crashpad_handler 把现场记在这里准备上传；文件夹里有 settings.dat、metadata、reports、attachments"
lose = "只是丢掉以前没传出去的崩溃记录，不影响使用；程序下次启动会自己建一个新的空记录库（换一个匿名编号）。"
how = "delete"
how_detail = "直接删除即可；程序正在运行时个别文件被占用，跳过就行。"
refs = ["https://chromium.googlesource.com/crashpad/crashpad/+/HEAD/doc/overview_design.md"]
verified_on = "2026-11-02"
```

**chromium-generic-cache、chromium-generic-code-cache、chromium-generic-gpu-cache、firefox-cache**：不替换，只在 `tier = "safe"` 下面加一行：
- chromium-generic-cache、firefox-cache：`lose_kind = "redownload"`
- chromium-generic-code-cache、chromium-generic-gpu-cache：`lose_kind = "slower"`

### 4.2 windows-system.toml

**user-temp（替换）**：暂时只让 Windows 自己清；引擎支持 `min_age_days` 后，见 4.6。

```toml
[[rule]]
id = "user-temp"
owner = "Windows"
category = "cache"
tier = "safe"
match = ['%TEMP%', '%LOCALAPPDATA%\Temp']
what = "你账户下的临时文件"
why = "软件运行、解压、安装时放中间文件的地方，本该用完就删，但很多软件会遗留"
lose = "7 天以上没动过的一般没人再用；最近几天的可能是：正在运行的程序（比如 Claude Code 会话）的文件、装到一半或要等重启才装完的安装包、已经下载好等下次启动安装的软件更新。这些删了要重下，有的回不来。"
how = "builtin_tool"
how_detail = "用“设置 > 系统 > 存储 > 临时文件”或“磁盘清理”：Windows 只删它判断没人在用的文件（磁盘清理默认只删 7 天没访问过的）。"
refs = [
  "https://support.microsoft.com/en-us/windows/free-up-drive-space-in-windows-85529ccb-c365-490d-b548-831022bc9b32",
  "https://learn.microsoft.com/windows-server/administration/windows-commands/cleanmgr",
]
verified_on = "2026-11-02"
```

**windows-temp（替换）**

```toml
[[rule]]
id = "windows-temp"
owner = "Windows"
category = "cache"
tier = "safe"
match = ['%SystemRoot%\Temp']
what = "系统级临时文件"
why = "系统服务、安装程序运行时留下的中间文件"
lose = "7 天以上没动过的一般没人再用；最近的可能属于正在进行或等重启才完成的安装（本机实测：vminst.log 还在写）。"
how = "builtin_tool"
how_detail = "用“设置 > 系统 > 存储 > 临时文件”或“磁盘清理”；Windows 会跳过还在用的和最近的文件。"
refs = [
  "https://support.microsoft.com/en-us/windows/free-up-drive-space-in-windows-85529ccb-c365-490d-b548-831022bc9b32",
  "https://learn.microsoft.com/windows-server/administration/windows-commands/cleanmgr",
]
verified_on = "2026-11-02"
```

**thumbnail-cache（替换）+ explorer-cache-dir（新增）**

```toml
[[rule]]
id = "thumbnail-cache"
owner = "Windows 资源管理器"
category = "cache"
tier = "safe"
lose_kind = "slower"
kind = "file"
match = [
  '%LOCALAPPDATA%\Microsoft\Windows\Explorer\thumbcache_*.db',
  '%LOCALAPPDATA%\Microsoft\Windows\Explorer\iconcache_*.db',
]
what = "缩略图和图标缓存"
why = "资源管理器把图片、视频的缩略图和程序图标存下来，打开文件夹时不用每次重新生成"
lose = "不丢数据；删掉后第一次打开图片多的文件夹会慢一点，缩略图和图标会重新生成。"
how = ["builtin_tool", "delete"]
how_detail = "用“磁盘清理”勾选“缩略图”最省事（这些文件被资源管理器占用，手动删常常删不掉）。"
refs = []
verified_on = "2026-11-02"

[[rule]]
id = "explorer-cache-dir"
owner = "Windows 资源管理器"
category = "system"
tier = "keep"
scope = "self"
kind = "dir"
match = ['%LOCALAPPDATA%\Microsoft\Windows\Explorer']
what = "缩略图和图标缓存所在的文件夹"
why = "里面的 thumbcache_*.db、iconcache_*.db 是缓存（单独标绿）；另外还有开始菜单的推荐过滤表和启动日志"
lose = "不要整个删，请点进去只清缩略图和图标缓存。"
how = "none"
how_detail = ""
refs = []
verified_on = "2026-11-02"
```

**inetcache（替换）**

```toml
[[rule]]
id = "inetcache"
owner = "Windows / IE 内核"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = [
  '%LOCALAPPDATA%\Microsoft\Windows\INetCache\IE',
  '%LOCALAPPDATA%\Microsoft\Windows\INetCache\Content.IE5',
]
what = "旧版 IE 内核的网页缓存"
why = "用到老式网页控件的程序会把网页文件缓存到这里"
lose = "不丢数据，网页下次联网重新下载。同一个文件夹下的 Content.Outlook（Outlook 打开附件时的副本）不在这一项里。"
how = ["builtin_tool", "delete"]
how_detail = "“磁盘清理”勾选“Internet 临时文件”，或直接删这两个子文件夹的内容。"
refs = []
verified_on = "2026-11-02"
```

**windows-update-download、delivery-optimization**：只加一行 `lose_kind = "redownload"`。

**directx-shader-cache**：只加一行 `lose_kind = "slower"`。

### 4.3 dev-caches.toml

**yarn-cache（替换）+ yarn-berry-cache（新增）**

```toml
[[rule]]
id = "yarn-cache"
owner = "Yarn 1"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%LOCALAPPDATA%\Yarn\Cache']
what = "Yarn 1 下载过的包缓存"
why = "让下次安装同一个包不用重新下载；项目里的 node_modules 是从这里复制过去的"
lose = "不丢数据；下次 yarn install 要联网重新下载。"
how = ["delete", "builtin_tool"]
how_detail = "yarn cache clean"
refs = ["https://classic.yarnpkg.com/en/docs/cli/cache"]
verified_on = "2026-11-02"

[[rule]]
id = "yarn-berry-cache"
owner = "Yarn 2+（Berry）"
category = "cache"
tier = "review"
lose_kind = "reinstall"
lose_short = "项目要重装依赖"
match = ['%LOCALAPPDATA%\Yarn\Berry\cache']
what = "Yarn 2 及以上的全局包缓存（zip 包）"
why = "新版 Yarn 默认让所有项目共用这一份缓存（enableGlobalCache）；用 PnP 的项目运行时直接从这里的 zip 读包，不再复制到项目里"
lose = "删了以后，用 PnP 的项目在重新 yarn install 之前都跑不起来，要一个个项目联网重装；用 node_modules 的项目只是下次安装要重新下载。"
how = ["builtin_tool", "delete"]
how_detail = "确认没有在用的 PnP 项目后再删；删完在每个项目里运行 yarn install。"
refs = ["https://yarnpkg.com/configuration/yarnrc#enableGlobalCache"]
verified_on = "2026-11-02"
```

**uv-cache（替换）**

```toml
[[rule]]
id = "uv-cache"
owner = "uv"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%LOCALAPPDATA%\uv\cache']
what = "uv 下载和解压过的 Python 包缓存"
why = "uv 把包缓存起来，建虚拟环境时直接从这里链接过去，所以特别快；uvx 和 uv run --with 的临时环境也在这里"
lose = "已经建好的虚拟环境一般不受影响；下次装包、跑 uvx 工具要联网重新下载。uv 官方说直接删缓存里的文件不安全（可能正好有 uv 在用），要用 uv 自己的命令清。"
how = "builtin_tool"
how_detail = "uv cache clean（全部清掉）或 uv cache prune（只清没用到的，官方说可以定期跑）。"
refs = ["https://docs.astral.sh/uv/concepts/cache/"]
verified_on = "2026-11-02"
```

**npm-cache（替换）**

```toml
[[rule]]
id = "npm-cache"
owner = "npm"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%LOCALAPPDATA%\npm-cache', '%APPDATA%\npm-cache']
what = "npm 下载过的包缓存"
why = "让下次安装同一个包不用重新下载；npx 临时运行的工具也放在这里（_npx）"
lose = "不丢数据；下次 npm install 要联网重新下载。npx 跑过的工具（包括用 npx 启动的 MCP 服务）也在这里，删了下次启动要联网重下，断网时启动不了。"
how = ["delete", "builtin_tool"]
how_detail = "npm cache clean --force （或直接删这个文件夹）。"
refs = ["https://docs.npmjs.com/cli/v10/commands/npm-cache"]
verified_on = "2026-11-02"
```

**nuget-cache（替换）**

```toml
[[rule]]
id = "nuget-cache"
owner = "NuGet（.NET）"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%USERPROFILE%\.nuget\packages', '%LOCALAPPDATA%\NuGet\v3-cache', '%LOCALAPPDATA%\NuGet\plugins-cache', '%LOCALAPPDATA%\NuGet\Cache']
what = "NuGet 下载的 .NET 包缓存"
why = ".NET 项目还原依赖时下载的包；.nuget\\packages 是项目编译时直接用的那一份，v3-cache 是 30 分钟就过期的下载缓存"
lose = "不丢代码；下次还原、编译要联网重新下载，离线时编译不了。Visual Studio 里可能要重新加载解决方案。"
how = ["builtin_tool", "delete"]
how_detail = "dotnet nuget locals all --clear （先关掉 Visual Studio）。"
refs = ["https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders"]
verified_on = "2026-11-02"
```

**cargo-registry（替换）**

```toml
[[rule]]
id = "cargo-registry"
owner = "Rust（cargo）"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%USERPROFILE%\.cargo\registry', '%USERPROFILE%\.cargo\git']
what = "cargo 下载的依赖源码和压缩包"
why = "编译 Rust 项目时下载的第三方库；cargo 编译时直接读这里的源码"
lose = "不丢数据；下次编译要联网重新下载依赖，离线时编译不了。"
how = "delete"
how_detail = "直接删；想只清旧的可以装 cargo-cache 工具。"
refs = ["https://doc.rust-lang.org/cargo/guide/cargo-home.html"]
verified_on = "2026-11-02"
```

**go-cache（替换）**

```toml
[[rule]]
id = "go-cache"
owner = "Go"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%LOCALAPPDATA%\go-build', '%USERPROFILE%\go\pkg\mod']
what = "Go 的编译缓存和下载的模块"
why = "go-build 是编译结果缓存，pkg\\mod 是下载的依赖源码"
lose = "不丢数据；go-build 删了下次编译慢（重新编译）；pkg\\mod 删了要联网重新下载，离线时编译不了。"
how = "builtin_tool"
how_detail = "go clean -cache （编译缓存）和 go clean -modcache （模块缓存，里面文件是只读的，手动删会报权限错误）。"
refs = ["https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching"]
verified_on = "2026-11-02"
```

**gradle-cache（替换）**

```toml
[[rule]]
id = "gradle-cache"
owner = "Gradle"
category = "cache"
tier = "safe"
lose_kind = "redownload"
match = ['%USERPROFILE%\.gradle\caches', '%USERPROFILE%\.gradle\wrapper\dists']
what = "Gradle 的依赖缓存和下载的 Gradle 版本"
why = "Android / Java 项目构建时下载的库和构建工具"
lose = "不丢代码；下次构建要联网重新下载依赖和 Gradle 本身，可能要很久，离线时构建不了。"
how = "delete"
how_detail = "关掉 Android Studio / IDE 和 Gradle 守护进程（gradle --stop）后删除。"
refs = ["https://docs.gradle.org/current/userguide/directory_layout.html"]
verified_on = "2026-11-02"
```

**vscode-family-user（替换）、vscode-family-root（替换）**：把 Kiro、Qoder 加进来，只加保护，不加可删项。

```toml
[[rule]]
id = "vscode-family-user"
owner = "VS Code 系编辑器"
category = "userdata"
tier = "keep"
owner_from_path = true
match = ['%APPDATA%\Code\User', '%APPDATA%\Code - Insiders\User', '%APPDATA%\Cursor\User', '%APPDATA%\Trae\User', '%APPDATA%\Windsurf\User', '%APPDATA%\Antigravity\User', '%APPDATA%\VSCodium\User', '%APPDATA%\Kiro\User', '%APPDATA%\Qoder\User']
what = "编辑器的个人设置、快捷键、代码片段和工作区记录"
why = "你在编辑器里改过的所有设置都在这里；User\\History 是本地历史（Timeline 里的旧版本）"
lose = "删了设置、快捷键、代码片段会丢（开了设置同步的可以同步回来），本地历史找不回来。里面的 workspaceStorage / History 比较大时可以单独考虑。"
how = "none"
how_detail = ""
refs = ["https://code.visualstudio.com/docs/configure/settings"]
verified_on = "2026-11-02"

[[rule]]
id = "vscode-family-root"
owner = "VS Code 系编辑器"
category = "userdata"
tier = "review"
lose_kind = "reinstall"
lose_short = "编辑器设置会丢"
owner_from_path = true
match = ['%APPDATA%\Code', '%APPDATA%\Code - Insiders', '%APPDATA%\Cursor', '%APPDATA%\Trae', '%APPDATA%\Windsurf', '%APPDATA%\Antigravity', '%APPDATA%\VSCodium', '%APPDATA%\Kiro', '%APPDATA%\Qoder']
what = "编辑器的数据文件夹（设置、扩展数据、缓存混在一起）"
why = "VS Code 系编辑器把设置、扩展存的数据（globalStorage）、没保存文件的备份（Backups）、内置浏览器缓存都放在这里"
lose = "整个删会丢设置、扩展数据和没保存的文件（Backups）。里面的 CachedData、CachedExtensionVSIXs、logs、Cache 类子文件夹可以清（会标成绿色）。"
how = "manual"
how_detail = "点进去只清缓存类子文件夹。"
refs = ["https://code.visualstudio.com/docs/editing/codebasics"]
verified_on = "2026-11-02"
```

**pnpm-store、bun-cache、playwright-browsers、electron-download-cache、pip-cache、conda-pkgs、vscode-family-cached**：只加一行 `lose_kind = "redownload"`。

**jetbrains-caches**：只加一行 `lose_kind = "slower"`。

### 4.4 cn-apps.toml

**wechat-render-cache（删掉整块）**，换成下面的 **wechat-radium（新增）**。

how 只写 builtin_tool，是故意的：这样清理页不会把整个 radium 当成一个可勾选项，会继续往里看，里面真正的 Chromium 缓存（Cache、Code Cache、GPUCache，约 150 MB）照样单独标成 safe。

```toml
[[rule]]
id = "wechat-radium"
owner = "微信"
category = "userdata"
tier = "review"
lose_kind = "unsure"
lose_short = "小程序数据会丢"
match = ['%APPDATA%\Tencent\xwechat\radium', '%APPDATA%\Tencent\WeChat\radium']
what = "微信内置浏览器和小程序的运行数据"
why = "微信 4.x 的 radium 里有内置浏览器的完整资料（Cookie、历史、Local Storage、IndexedDB）、小程序的代码包和本地存储（applet\\local\\<小程序>\\usr、mmkv）、设备编号 device_uuid_0；真正的网页缓存（Cache、Code Cache、GPUCache）在 web\\profiles 里，会单独标绿"
lose = "不丢聊天记录；但公众号文章和网页要重新登录，小程序存在本机的数据（草稿、设置、登录状态）会丢，只存在本机的找不回来。"
how = "builtin_tool"
how_detail = "用微信“设置 → 存储空间 → 清理缓存”；不要手动删整个文件夹。"
refs = []
verified_on = "2026-11-02"
```

**wechat-logs（替换）**：加上 `xplayer\dump`。

```toml
[[rule]]
id = "wechat-logs"
owner = "微信"
category = "log"
tier = "safe"
match = [
  '%APPDATA%\Tencent\xwechat\log',
  '%APPDATA%\Tencent\xwechat\crashinfo',
  '%APPDATA%\Tencent\xwechat\xplayer\dump',
  '%APPDATA%\Tencent\WeChat\log',
  '%APPDATA%\Tencent\WeChat\xweb\crash',
]
what = "微信的运行日志和崩溃记录"
why = "微信记录运行情况，出问题时给客服排查"
lose = "只是丢掉日志，不影响聊天记录和使用。"
how = "delete"
how_detail = "退出微信后删除。"
refs = []
verified_on = "2026-11-02"
```

**netease-music-logs（替换）**：去掉 dumps，让它继承 netease-music-data（review）。

```toml
[[rule]]
id = "netease-music-logs"
owner = "网易云音乐"
category = "log"
tier = "safe"
match = ['%LOCALAPPDATA%\NetEase\CloudMusic\Log', '%LOCALAPPDATA%\NetEase\CloudMusic\cloudmusic.elog']
what = "网易云音乐的日志"
why = "程序运行记录，出问题时排查用"
lose = "只是丢掉日志，不影响使用。"
how = "delete"
how_detail = "退出网易云音乐后删除。"
refs = []
verified_on = "2026-11-02"
```

**wechat-update、feishu-update、netease-music-cache**：只加一行 `lose_kind = "redownload"`。

### 4.5 misc.toml

- **outlook-ost**：两行替换成：
  ```toml
  lose_kind = "irreversible"
  lose_short = "没同步的邮件会丢"
  ```
  理由：规则自己的 lose 已经写了“还没同步上去的草稿等会丢”，按“取最坏的”应该是 irreversible。
- **gpu-shader-cache、adobe-media-cache**：只加一行 `lose_kind = "slower"`。
- **electron-updater-cache、steam-caches、raspberry-pi-imager-cache**：只加一行 `lose_kind = "redownload"`。
- **ai-models.toml 的 ollama-updates**：只加一行 `lose_kind = "redownload"`。

### 4.6 引擎上线 `min_age_days` 以后的 user-temp、windows-temp

**现在不能用**：RuleDef 是 `deny_unknown_fields`，多一个字段校验就过不了。引擎支持后再改：

```toml
# user-temp / windows-temp 在上面版本的基础上改两行：
how = ["delete", "builtin_tool"]
min_age_days = 7
```

### 4.7 改完要同步的地方

- **`crates/diskwhy-core/tests/explain_tests.rs`**：
  - 286 行：`check("wechat-render-cache", Tier::Safe…)` 改成 `wechat-radium` / Review。
  - 305 行：`chromium-generic-sw-cache` 改成 Review。
  - 362–363 行：飞书 CacheStorage 断言是 safe，要改成 review。这正是本次要纠正的情况。
  - 约 1462 行：user-temp 候选组断言 `deletable_bytes == 80`，how 改成 builtin_tool 后变成 0。
- **`crates/diskwhy-core/src/explain/candidates.rs` 的 RULE_LABELS**：
  - 删掉 `wechat-render-cache`；
  - 加上 `("wechat-radium", "微信小程序数据")`、`("yarn-berry-cache", "Yarn 新版缓存")`、`("explorer-cache-dir", "资源管理器缓存")`。
- **`src/dev/fixture.json`**：界面假数据里的 id 也跟着改。

---

## 五、引擎和界面要补的东西

1. **`min_age_days = N`（按文件年龄挑，新功能）**
   - **规则写法**：只能用在 tree 规则上。
   - **清理页怎么列**：命中文件夹 F 后，F 不再整个算一个候选，而是把 F 下每个直接子项单独看：
     - 子项（文件夹的话看它整棵子树里最新的修改时间）N 天内都没改过，才列成可删；
     - 新一点的算“最近在用，不删”，字节单独显示。
   - **执行时**：预检和执行用同一个年龄门槛再查一遍，就像现在的 30 分钟“刚改过”检查。
   - **需要的规则**：
     - user-temp、windows-temp（N = 7，和磁盘清理一致）；
     - 以后可以给 squirrel-temp、root-tmp-dir、root-temp-dir 也用上。
   - **为什么 when_child 不够**：when_child 只看下一层有没有某个名字，表达不了“最近有没有人在用”。本机 4 GB 临时文件里的风险全在“最近 7 天”这一维上。
   - **代价**：扫描树里文件夹节点的 mtime 目前是文件夹自己的时间，要另外记子树里最新的修改时间。
2. **safe 行也显示后果标签**
   - **现状**：
     - rules.rs 的 validate 只要求 review 必须写 lose_kind，safe 可以写，写了就会传进候选组（candidates.rs 的 add_rule）；
     - 但 `src/pages/CleanupPage.vue` 第 173 和 187 行只给 `tier === 'review'` 画标签；
     - safe 那一节的提示写死了“删了会自动重建”（第 155 行），对 npm、uv、VSIX、Ollama 更新包这类要联网才能回来的不对。
   - **建议**：
     - safe 行的 lose_kind 是 redownload 时也显示标签（“要重新下载”）；
     - 提示改成“删了会自己长回来或重新下载”。
   - **影响**：所有补了 redownload 的 safe 规则。
3. **when_child 要能看到被合并的小文件**（小改动）
   - 现在 when_child 只看扫描树里保留的前 50 个文件。标记文件很小、旁边又有 50 个以上更大的文件时，标记会被并进“其余小文件”，规则就不命中。
   - 这个方向是“少删”，不危险，但会漏掉真的缓存。
   - 建议像启发式那样，没找到时再问一次文件系统。
4. **不需要 `unless_child`（反向条件）**
   - 这次所有需要“排除”的地方都能用别的办法做：
     - 把模式写窄（inetcache、thumbnail-cache）；
     - 让更具体的规则胜出（radium 的 how 只写 builtin_tool，清理页就会往里找真缓存）。
   - 本机 43 个 Cache 命中里，没有一个是“带着标记、里面却混着用户数据”的，所以现在没有 `unless_child` 才能解决的情况。
   - 如果以后发现某个 Electron 程序在 Chromium 缓存目录里放自己的数据，再加：`unless_child = [...]`，意思是目录里只要有其中任意一个名字就不算命中，优先级比 when_child 高。

---

## 六、没能核实的

- **微信 radium 各子目录到底是谁读、删了怎么回来**：没有官方文档。微信小程序存储那页抓下来只有导航，正文没抓到。这次的判断依据是本机文件夹里的实际内容，所以没给它新建任何 safe 子规则。
- **Outlook `Content.Outlook` 的行为**：本机没装经典 Outlook，也没抓官方文档。只是把它排除在 safe 之外，没有给它写新规则。
- **本机没有、只按官方文档判断的**：Yarn Berry、NuGet、JetBrains、Firefox、Steam、Adobe、Go、Gradle、pnpm、bun，以及 C 盘上的 conda。
- **不提权读不了的**：delivery-optimization、LiveKernelReports、Prefetch 等 59 个目录，大小和内容都没看到。
- **ScriptCache 被删但注册信息还在时，Chromium 会不会自动注销重建**：没有查源码，规则文案里写的是“可能”。
- **网易云 `dumps\cookie_json` 的用途**：不清楚，所以才建议把它移出 safe。
- **uv 在 Windows 上默认用硬链接还是复制**：没有核实。文案里只写“一般不受影响”。
- **磁盘清理“7 天”的默认值**：来自第三方资料和微软问答，微软 Learn 上没有找到直接写明的页面。

## 七、引用

- Chromium `browsing_data_remover.h`（CacheStorage 属于网站数据）：https://chromium.googlesource.com/chromium/src/+/HEAD/content/public/browser/browsing_data_remover.h
- MDN 存储配额与清除规则：https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria
- Crashpad 设计概览（settings.dat 里存客户端编号和上传开关）：https://chromium.googlesource.com/crashpad/crashpad/+/HEAD/doc/overview_design.md
- VS Code 热退出和 Backups：https://code.visualstudio.com/docs/editing/codebasics
- Yarn `enableGlobalCache`，以及 PnP 直接从 zip 读包：https://yarnpkg.com/configuration/yarnrc
- NuGet 的 global-packages 和 http-cache：https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders
- uv 缓存（直接改缓存永远不安全）：https://docs.astral.sh/uv/concepts/cache/
- Windows 应用临时数据（随时可能被删）：https://learn.microsoft.com/en-us/windows/apps/develop/data/store-and-retrieve-app-data
- cleanmgr：https://learn.microsoft.com/windows-server/administration/windows-commands/cleanmgr
- 磁盘清理 LastAccess=7（第三方）：https://www.winhelponline.com/blog/disk-cleanup-wipe-out-temp-folder-contents-completely-lastaccess/
