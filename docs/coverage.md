# 解释覆盖率

## 2026-09-24 复核（逐条核对之后）

日期：2026-09-24　机器：作者本机 Windows 11 Pro 10.0.26200，**普通用户权限**。C 盘 NTFS 199 GB（已用 183.65 GB），D 盘 754 GB（已用 702.36 GB）。
快照：`diskwhy-cli scan C:\ --snapshot c.dwsnap`（71.5 秒）、`scan D:\ --snapshot d.dwsnap`（50.5 秒）；以下数字都来自 `explain --from <快照> --top N`。规则库 **213 条**，注册表已安装软件 260 个。

**这次不只数“有没有说明”，而是把 C 盘前 120 条逐条拿去和真实内容、官方文档对**，详见 [AUDIT-2026-09-24.md](AUDIT-2026-09-24.md)。一句话结论：修前覆盖率 100%，但 120 条里只有 82 条说对（❌ 10、⚠️ 28、❓ 0）；10 条错误全是同一个原因——商店版 Claude 的 AppData 重定向（`Packages\Claude_…\LocalCache\Local`，里面是别的程序写的文件）被说成了“Claude 的数据”，已从引擎层面修正。

| 范围 | 规则（精确 + 继承） | 启发式 | 未知 | 覆盖率 | 修前（同一份快照） |
|---|---:|---:|---:|---:|---|
| C 盘前 50 | 46（40 + 6） | 4 | 0 | **100%** | 100%（50 条规则，其中 5 条说错） |
| C 盘前 120 | 111（74 + 37） | 9 | 0 | **100%** | 100%（✅ 82 / ❌ 10 / ⚠️ 28 / ❓ 0） |
| C 盘前 200 | 183（95 + 88） | 17 | 0 | **100%** | 修前没单独跑 |
| D 盘前 40 | 0 | 40 | 0 | **100%** | **50.0%**（启发式 20、未知 20） |
| D 盘前 150 | 0 | 143 | 7 | **95.3%** | 修前没单独跑 |

怎么看这些数字：

1. **覆盖率不等于正确率。** 修前 C 盘前 120 覆盖率就是 100%，逐条核对才发现 10 条说错；修后只对改动过的 38 条逐一看了新输出，没有再做一轮完整复核。
2. **D 盘全靠启发式。** 规则的路径必须以 `%变量%` 开头，管不到用户自选的 `D:\steam`、`D:\conda`；这次新增的 `big_items.rs` 按名字和目录结构认聊天记录、游戏、虚拟机、conda、安卓 / Flutter SDK、编译产物、自装软件等（全部不会是 safe）。所以 D 盘“全盘按规则库归档”一栏全是“未覆盖”（那一栏不算启发式）。
3. **C 盘全盘归档的变化**：safe 29.65 → 31.47 GB（新增 Ollama 更新包、飞书日志两条 safe），review 59.72 → 49.47 GB，keep 67.55 → 68.71 GB，**规则未覆盖 13.95 → 21.22 GB**。未覆盖变多是因为原来约 7 GB 被错记在 `claude-desktop-data` 名下，现在按真实位置归档后规则库管不到（由启发式说明）——数字更对了，不是退步。
4. D 盘前 150 剩下的 7 个未知：`D:\jiedan`（及 `Shengcheng`、`Shengcheng\downloads`）、`D:\FeverApps`（及 `MCBedrock`、`MCBedrock\data`）、`D:\MCLDownload`。线索看起来分别是个人机器学习项目的数据、某个 Minecraft 基岩版启动器、网易我的世界启动器的下载目录，**都没有核实**，所以保持未知。
5. 仍是普通权限扫描；前 N 大目录是嵌套的（`C:\Users` ⊃ `C:\Users\king` ⊃ …），N 行不是 N 块不相交的空间。

### C 盘前 50（修后，CLI 原始输出）

#### 前 50 大目录逐项解释（`diskwhy-cli explain C:\ --top 50` 原始输出）

扫描 71.5 秒，卷已用 183.65 GB；规则 213 条，注册表已安装软件 260 个（读取 14 ms），解释 50 个目录用时 12 ms。

| # | 占用 | 档位 | 来源 | 路径 | 这是什么 |
|---:|---:|---|---|---|---|
| 1 | 109.77 GB | 保留 | 规则 users-root | `C:\Users` | 所有 Windows 账户的个人文件夹都在这里 |
| 2 | 109.77 GB | 保留 | 规则 user-profile | `C:\Users\king` | 你这个账户的个人文件夹 |
| 3 | 95.25 GB | 保留 | 规则 appdata-root | `C:\Users\king\AppData` | 软件存放个人设置、缓存和数据的总文件夹（默认隐藏） |
| 4 | 68.78 GB | 保留 | 规则 appdata-local | `C:\Users\king\AppData\Local` | 只属于这台电脑的软件数据（AppData\Local） |
| 5 | 25.76 GB | 保留 | 规则 windows-dir | `C:\Windows` | Windows 系统本身 |
| 6 | 22.98 GB | 保留 | 规则 program-files | `C:\Program Files` | 软件的安装目录（Program Files） |
| 7 | 20.04 GB | 保留 | 规则 appdata-roaming | `C:\Users\king\AppData\Roaming` | 软件的设置和个人数据（AppData\Roaming） |
| 8 | 19.07 GB | 保留 | 规则 store-apps-data | `C:\Users\king\AppData\Local\Packages` | 商店应用（MSIX 包）的数据，每个应用一个文件夹 |
| 9 | 17.68 GB | 需判断 | 规则 claude-desktop-package | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc` | 商店版 Claude 桌面版的应用包文件夹 |
| 10 | 17.68 GB | 需判断 | 规则 msix-localcache | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache` | 这个商店应用（及它启动的程序）写进 AppData 的文件，被 Windows 存到了应用包里 |
| 11 | 11.02 GB | 保留 | 规则 system32 | `C:\Windows\System32` | Windows 的核心程序和系统库（SysWOW64 是给 32 位程序用的那一套） |
| 12 | 10.12 GB | 需判断 | 规则 msix-localcache | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming` | 这个商店应用（及它启动的程序）写进 AppData 的文件，被 Windows 存到了应用包里 |
| 13 | 10.09 GB | 需判断 | 规则 claude-desktop-data | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` | Claude 桌面版的本地数据：登录信息、设置、界面缓存、Cowork 虚拟机 |
| 14 | 9.19 GB | 保留 | 规则 programdata-root | `C:\ProgramData` | 所有账户共用的软件数据（ProgramData，默认隐藏） |
| 15 | 8.18 GB | 需判断 | 规则 claude-desktop-vm | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles` | Claude 桌面版 Cowork 功能在本机跑的 Linux 虚拟机（“工作区”） |
| 16 | 8.18 GB | 需判断 | 规则 claude-desktop-vm（继承上层） | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle` | Claude 桌面版 Cowork 功能在本机跑的 Linux 虚拟机（“工作区”） |
| 17 | 7.66 GB | 保留 | 规则 windows-apps | `C:\Program Files\WindowsApps` | 商店应用（Microsoft Store 应用）的程序本体 |
| 18 | 7.60 GB | 需判断 | 规则 docker-local | `C:\Users\king\AppData\Local\Docker` | Docker Desktop 的本机数据（虚拟磁盘、日志、安装记录） |
| 19 | 7.57 GB | 保留 | 规则 docker-wsl | `C:\Users\king\AppData\Local\Docker\wsl` | Docker Desktop 的 WSL 虚拟磁盘文件夹 |
| 20 | 7.56 GB | 需判断 | 规则 msix-localcache | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local` | 这个商店应用（及它启动的程序）写进 AppData 的文件，被 Windows 存到了应用包里 |
| 21 | 7.48 GB | 保留 | 规则 docker-data-vhdx | `C:\Users\king\AppData\Local\Docker\wsl\disk` | Docker Desktop 的数据盘：所有镜像、容器、数据卷都在这一个虚拟磁盘文件里 |
| 22 | 7.01 GB | 需判断 | 启发式 rust-target | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target` | Rust 项目的编译产物（cargo 的 target 文件夹） |
| 23 | 7.01 GB | 需判断 | 启发式 dominant-child | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build` | 里面几乎全是「target」：Rust 项目的编译产物（cargo 的 target 文件夹） |
| 24 | 6.44 GB | 保留 | 规则 local-programs | `C:\Users\king\AppData\Local\Programs` | 只给你这个账户安装的软件（不需要管理员权限的那种） |
| 25 | 6.30 GB | 需判断 | 规则 aclos-game-recorder | `C:\Users\king\AppData\ACLOS` | 《无畏契约》国服配套程序（ACLOS）的数据，以及它自动录下的“无畏时刻”视频 |
| 26 | 6.30 GB | 需判断 | 规则 aclos-highlight-videos | `C:\Users\king\AppData\ACLOS\aclos-highlight` | 《无畏契约》“无畏时刻”自动录下的精彩片段视频 |
| 27 | 6.11 GB | 保留 | 规则 google-local | `C:\Users\king\AppData\Local\Google` | Google 家软件（主要是 Chrome）的本机数据 |
| 28 | 6.11 GB | 保留 | 规则 google-local | `C:\Users\king\AppData\Local\Google\Chrome` | Google 家软件（主要是 Chrome）的本机数据 |
| 29 | 6.11 GB | 保留 | 规则 chrome-profile | `C:\Users\king\AppData\Local\Google\Chrome\User Data` | Chrome 的用户资料：书签、密码、历史记录、扩展、登录状态 |
| 30 | 6.07 GB | 保留 | 规则 driverstore | `C:\Windows\System32\DriverStore` | 驱动仓库：装过的所有驱动安装包 |
| 31 | 6.06 GB | 保留 | 规则 driverstore（继承上层） | `C:\Windows\System32\DriverStore\FileRepository` | 驱动仓库：装过的所有驱动安装包 |
| 32 | 5.49 GB | 需判断 | 规则 aclos-highlight-videos（继承上层） | `C:\Users\king\AppData\ACLOS\aclos-highlight\wonderfulVideos690838325698893058` | 《无畏契约》“无畏时刻”自动录下的精彩片段视频 |
| 33 | 5.42 GB | 可清理 | 规则 user-temp | `C:\Users\king\AppData\Local\Temp` | 你账户下的临时文件 |
| 34 | 5.29 GB | 保留 | 规则 wsl-root | `C:\Users\king\AppData\Local\wsl` | 新版 WSL 存放 Linux 发行版虚拟磁盘的文件夹 |
| 35 | 5.29 GB | 保留 | 规则 wsl-distro-vhdx | `C:\Users\king\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}` | WSL 里某个 Linux 发行版的整块硬盘（ext4.vhdx） |
| 36 | 5.13 GB | 需判断 | 启发式 rust-target | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug` | Rust 项目的编译产物（cargo 的 target 文件夹） |
| 37 | 4.93 GB | 需判断 | 规则 feishu-larkshell | `C:\Users\king\AppData\Roaming\LarkShell` | 飞书的本地数据：各账号的聊天和文档缓存、内置浏览器数据、登录信息、更新包 |
| 38 | 4.79 GB | 保留 | 规则 nvidia-programdata | `C:\ProgramData\NVIDIA Corporation` | NVIDIA 驱动和软件的公共数据 |
| 39 | 4.59 GB | 需判断 | 规则 nvidia-app-data | `C:\ProgramData\NVIDIA Corporation\NVIDIA App` | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包） |
| 40 | 4.53 GB | 需判断 | 规则 nvidia-app-data（继承上层） | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework` | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包） |
| 41 | 4.53 GB | 需判断 | 规则 nvidia-app-updates | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts` | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包 |
| 42 | 4.51 GB | 保留 | 规则 office-program | `C:\Program Files\Microsoft Office` | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体 |
| 43 | 4.50 GB | 保留 | 规则 office-program（继承上层） | `C:\Program Files\Microsoft Office\root` | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体 |
| 44 | 3.98 GB | 需判断 | 规则 chrome-on-device-model | `C:\Users\king\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel` | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用） |
| 45 | 3.98 GB | 需判断 | 规则 chrome-on-device-model（继承上层） | `C:\Users\king\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel\2025.8.8.1141` | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用） |
| 46 | 3.81 GB | 保留 | 规则 windows-installer-cache | `C:\Windows\Installer` | 已装软件的安装包缓存（.msi / .msp） |
| 47 | 3.78 GB | 需判断 | 启发式 rust-target | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug\deps` | Rust 项目的编译产物（cargo 的 target 文件夹） |
| 48 | 3.68 GB | 需判断 | 规则 nvidia-driver-installers | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts\grd` | NVIDIA App 下载的显卡驱动安装包（grd = Game Ready 驱动，crd = Studio 驱动） |
| 49 | 3.65 GB | 保留 | 规则 winsxs | `C:\Windows\WinSxS` | Windows 组件仓库：系统组件和更新的所有版本 |
| 50 | 3.60 GB | 需判断 | 规则 claude-code-data | `C:\Users\king\.claude` | Claude Code 的配置、会话记录和插件 |

##### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 40 |
| 规则继承上层 | 6 |
| 启发式 | 4 |
| 未知 | 0 |
| **规则 + 启发式覆盖率** | **100.0%**（50/50） |

##### 全盘按规则库归档（不含启发式）

| 档位 | 字节 |
|---|---:|
| 可清理 safe | 31.47 GB |
| 需判断 review | 49.47 GB |
| 保留 keep | 68.71 GB |
| 规则未覆盖 | 21.22 GB |

命中过的规则 106/213。safe 档最大的几项：
- `user-temp` 5.42 GB：你账户下的临时文件
- `chromium-generic-sw-cache` 3.38 GB：网页离线缓存（Service Worker 缓存）
- `chromium-generic-cache` 3.25 GB：网页缓存（Chromium 内核程序通用格式）
- `electron-updater-cache` 2.53 GB：软件自动更新时下载下来的安装包
- `gpu-shader-cache` 1.65 GB：显卡驱动的着色器缓存
- `npm-cache` 1.53 GB：npm 下载过的包缓存
- `chromium-generic-code-cache` 1.50 GB：脚本编译缓存（Chromium 内核程序通用）
- `vscode-family-cached` 1.48 GB：编辑器的缓存和日志（旧版本的编译缓存、扩展安装包副本、运行日志）

##### 未知项（下一批规则的待办）

（无）

### D 盘前 40（修后，CLI 原始输出）

#### 前 40 大目录逐项解释（`diskwhy-cli explain D:\ --top 40` 原始输出）

扫描 50.5 秒，卷已用 702.36 GB；规则 213 条，注册表已安装软件 260 个（读取 13 ms），解释 40 个目录用时 88 ms。

| # | 占用 | 档位 | 来源 | 路径 | 这是什么 |
|---:|---:|---|---|---|---|
| 1 | 217.21 GB | 保留 | 启发式 game-tencent-root | `D:\valorant_game\Tencent Games` | 腾讯游戏的文件夹：游戏本体 + 游戏自动录的精彩片段视频 |
| 2 | 217.21 GB | 保留 | 启发式 game-tencent-root | `D:\valorant_game` | 腾讯游戏的文件夹：游戏本体 + 游戏自动录的精彩片段视频 |
| 3 | 182.09 GB | 需判断 | 启发式 game-highlights | `D:\valorant_game\Tencent Games\wonderfulVideos6192787854092623945` | 游戏自动录下来的精彩片段视频 |
| 4 | 155.67 GB | 保留 | 启发式 steam-root | `D:\steam` | Steam 客户端和它的游戏库 |
| 5 | 153.10 GB | 保留 | 启发式 steam-library | `D:\steam\steamapps` | Steam 游戏库（Steam 下载的所有游戏都在这里） |
| 6 | 151.85 GB | 保留 | 启发式 steam-library | `D:\steam\steamapps\common` | Steam 游戏库里所有游戏的安装文件夹 |
| 7 | 57.68 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RESIDENT EVIL 4  BIOHAZARD RE4` | Steam 游戏「RESIDENT EVIL 4  BIOHAZARD RE4」的安装文件 |
| 8 | 51.55 GB | 保留 | 启发式 conda-root | `D:\conda` | conda 的主文件夹（环境 envs + 包缓存 pkgs） |
| 9 | 49.00 GB | 保留 | 启发式 conda-envs | `D:\conda\envs` | conda 的环境集合（15 个环境） |
| 10 | 42.58 GB | 保留 | 启发式 user-projects | `D:\desk` | 放代码项目的文件夹（找到 19 个项目） |
| 11 | 35.12 GB | 保留 | 启发式 game-tencent | `D:\valorant_game\Tencent Games\VALORANT\live` | 腾讯游戏「VALORANT」的安装文件 |
| 12 | 35.12 GB | 保留 | 启发式 game-tencent | `D:\valorant_game\Tencent Games\VALORANT` | 腾讯游戏「VALORANT」的安装文件 |
| 13 | 34.72 GB | 保留 | 启发式 game-tencent | `D:\valorant_game\Tencent Games\VALORANT\live\ShooterGame` | 腾讯游戏「VALORANT」的安装文件 |
| 14 | 33.05 GB | 保留 | 启发式 game-tencent | `D:\valorant_game\Tencent Games\VALORANT\live\ShooterGame\Content` | 腾讯游戏「VALORANT」的安装文件 |
| 15 | 32.95 GB | 保留 | 启发式 game-tencent | `D:\valorant_game\Tencent Games\VALORANT\live\ShooterGame\Content\Paks` | 腾讯游戏「VALORANT」的安装文件 |
| 16 | 24.14 GB | 保留 | 启发式 chat-files | `D:\wxliaotianjilu\xwechat_files` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 17 | 24.14 GB | 保留 | 启发式 chat-files-container | `D:\wxliaotianjilu` | 放微信聊天记录的文件夹 |
| 18 | 22.88 GB | 保留 | 启发式 chat-files | `D:\wxliaotianjilu\xwechat_files\wxid_2zteudzwcocl22_2ad0` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 19 | 22.69 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 20 | 22.57 GB | 需判断 | 启发式 dev-sdk-container | `D:\desk\sdk` | 放开发工具包（SDK）和它们缓存的文件夹 |
| 21 | 22.42 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers\resources` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 22 | 22.42 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers\resources\app` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 23 | 22.41 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers\resources\app\src` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 24 | 22.41 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers\resources\app\src\res` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 25 | 22.41 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\RevengeOnGoldDiggers\resources\app\src\res\hash` | Steam 游戏「RevengeOnGoldDiggers」的安装文件 |
| 26 | 21.84 GB | 保留 | 启发式 chat-files | `D:\wxliaotianjilu\xwechat_files\wxid_2zteudzwcocl22_2ad0\msg` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 27 | 20.85 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\Aim Lab` | Steam 游戏「Aim Lab」的安装文件 |
| 28 | 20.69 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\Aim Lab\AimLab_tb_Data` | Steam 游戏「Aim Lab」的安装文件 |
| 29 | 18.99 GB | 保留 | 启发式 user-projects | `D:\GPT-SoVITS-v2pro-20250604` | 一个代码项目 |
| 30 | 14.60 GB | 保留 | 启发式 chat-files | `D:\wechat_history\xwechat_files` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 31 | 14.60 GB | 保留 | 启发式 chat-files-container | `D:\wechat_history` | 放微信聊天记录的文件夹 |
| 32 | 14.16 GB | 保留 | 启发式 chat-files | `D:\wechat_history\xwechat_files\wxid_2zteudzwcocl22_2ad0` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 33 | 12.97 GB | 保留 | 启发式 chat-files | `D:\wechat_history\xwechat_files\wxid_2zteudzwcocl22_2ad0\msg` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 34 | 12.62 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\Aim Lab\AimLab_tb_Data\StreamingAssets` | Steam 游戏「Aim Lab」的安装文件 |
| 35 | 11.95 GB | 保留 | 启发式 conda-env | `D:\conda\envs\graph312` | conda 环境「graph312」（11.95 GB） |
| 36 | 11.23 GB | 保留 | 启发式 user-projects | `D:\desk\claudex` | 放代码项目的文件夹（找到 7 个项目） |
| 37 | 10.58 GB | 保留 | 启发式 steam-game | `D:\steam\steamapps\common\Disco Elysium` | Steam 游戏「Disco Elysium」的安装文件 |
| 38 | 10.31 GB | 保留 | 启发式 user-projects | `D:\ideas` | 放代码项目的文件夹（找到 5 个项目） |
| 39 | 9.96 GB | 保留 | 启发式 chat-files | `D:\wxliaotianjilu\xwechat_files\wxid_2zteudzwcocl22_2ad0\msg\file` | 微信的聊天记录和聊天文件（收到的图片、视频、文件） |
| 40 | 9.75 GB | 需判断 | 启发式 model-weights | `D:\ideas\sga_models` | 看起来是 AI 模型文件（模型权重） |

##### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 0 |
| 规则继承上层 | 0 |
| 启发式 | 40 |
| 未知 | 0 |
| **规则 + 启发式覆盖率** | **100.0%**（40/40） |

##### 全盘按规则库归档（不含启发式）

| 档位 | 字节 |
|---|---:|
| 可清理 safe | 0 B |
| 需判断 review | 0 B |
| 保留 keep | 0 B |
| 规则未覆盖 | 681.90 GB |

命中过的规则 0/213。safe 档最大的几项：

##### 未知项（下一批规则的待办）

（无）

---

## 2026-09-23 阶段 B 的原始记录（保留作对照；其中商店版 Claude 应用包 Packages\Claude_… 内各条目的说明已在 2026-09-24 审计中更正，见 AUDIT-2026-09-24.md）

日期：2026-09-23　机器：作者本机 Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
命令：`target\release\diskwhy-cli.exe explain C:\ --top 50 --quiet --md ... --json ...`（另跑了一次 `--top 200` 找待办）
规则库：179 条（6 个分组），注册表 Uninstall 快照 260 个已安装软件。

### 结论

| 范围 | 规则（精确 + 继承上层） | 启发式 | 未知 | 规则 + 启发式覆盖率 |
|---|---:|---:|---:|---:|
| 前 50 大目录（PLAN 成功标准的口径） | 47（34 + 13） | 3 | 0 | **100.0%**（目标 ≥ 80%） |
| 前 200 大目录（找待办用） | 172（80 + 92） | 19 | 9 | **95.5%** |

#### 这个数字要怎么看（诚实说明）

1. **“容器”也算命中**：前 50 里有 15 个是 `scope = "self"` 的容器规则（`C:\Users`、`AppData\Local`、`Program Files`、`ProgramData`、`Packages` 等）。它们确实回答了“这是什么”，但回答是“里面混着各种东西，点进去看”。把这 15 个去掉，剩下 35 个具体目录也全部有规则或启发式说明。
2. **继承命中**：13 个是“上层文件夹命中了规则、子文件夹沿用”，比如回收站里的子目录、`Claude_…\LocalCache`。界面和 CLI 都会标“继承上层”，不会假装是精确命中。
3. **3 个启发式全是同一处**：`C:\Users\king\AppData\ACLOS\…`（6.3 GB 的 .mp4/.jpeg，名字叫 wonderfulVideos）。注册表里找不到对应软件，所以没有写规则；启发式 `media-files` 给出的是“一堆视频/图片/音频，可能是你自己的，也可能是软件自动录的，先打开看看”，档位 review。它是否算“有说明”见仁见智——如果不算，前 50 覆盖率是 94%。
4. **前 50 大目录是嵌套的**：`C:\Users` ⊃ `C:\Users\king` ⊃ `AppData` …，所以 50 行并不是 50 块不相交的空间。
5. **全盘按规则库归档**（safe / review / keep / 未覆盖）四项之和大于卷已用空间，因为阶段 A 已知的约 10 GB 重复计数（WinSxS 硬链接、商店版 Claude 目录映射）还没去重（阶段 C）。
6. 启发式和规则都**只看名字、大小、时间**，没有打开任何文件读内容。

### 附：前 50 大目录逐项解释（`diskwhy-cli explain C:\ --top 50` 原始输出）

扫描 40.9 秒，卷已用 183.33 GB；规则 179 条，注册表已安装软件 260 个（读取 13 ms），解释 50 个目录用时 10 ms。

| # | 占用 | 档位 | 来源 | 路径 | 这是什么 |
|---:|---:|---|---|---|---|
| 1 | 116.67 GB | 保留 | 规则 users-root | `C:\Users` | 所有 Windows 账户的个人文件夹都在这里 |
| 2 | 116.66 GB | 保留 | 规则 user-profile | `C:\Users\king` | 你这个账户的个人文件夹 |
| 3 | 102.78 GB | 保留 | 规则 appdata-root | `C:\Users\king\AppData` | 软件存放个人设置、缓存和数据的总文件夹（默认隐藏） |
| 4 | 65.65 GB | 保留 | 规则 appdata-local | `C:\Users\king\AppData\Local` | 只属于这台电脑的软件数据（AppData\Local） |
| 5 | 36.31 GB | 保留 | 规则 windows-dir | `C:\Windows` | Windows 系统本身 |
| 6 | 30.70 GB | 保留 | 规则 appdata-roaming | `C:\Users\king\AppData\Roaming` | 软件的设置和个人数据（AppData\Roaming） |
| 7 | 15.95 GB | 保留 | 规则 program-files | `C:\Program Files` | 软件的安装目录（Program Files） |
| 8 | 12.38 GB | 保留 | 规则 system32 | `C:\Windows\System32` | Windows 的核心程序和系统库（SysWOW64 是给 32 位程序用的那一套） |
| 9 | 12.02 GB | 保留 | 规则 store-apps-data | `C:\Users\king\AppData\Local\Packages` | 商店应用（MSIX 包）的数据，每个应用一个文件夹 |
| 10 | 11.99 GB | 保留 | 规则 winsxs | `C:\Windows\WinSxS` | Windows 组件仓库：系统组件和更新的所有版本 |
| 11 | 10.62 GB | 需判断 | 规则 claude-desktop-data | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc` | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像 |
| 12 | 10.62 GB | 需判断 | 规则 claude-desktop-data（继承上层） | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache` | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像 |
| 13 | 10.28 GB | 需判断 | 规则 recycle-bin | `C:\$Recycle.Bin` | 回收站：你删掉的文件其实还在这里 |
| 14 | 10.28 GB | 需判断 | 规则 recycle-bin（继承上层） | `C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001` | 回收站：你删掉的文件其实还在这里 |
| 15 | 10.28 GB | 需判断 | 规则 recycle-bin（继承上层） | `C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV` | 回收站：你删掉的文件其实还在这里 |
| 16 | 10.09 GB | 需判断 | 规则 claude-desktop-data（继承上层） | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming` | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像 |
| 17 | 10.06 GB | 需判断 | 规则 claude-desktop-data（继承上层） | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像 |
| 18 | 10.06 GB | 需判断 | 规则 claude-desktop-data | `C:\Users\king\AppData\Roaming\Claude` | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像 |
| 19 | 9.58 GB | 需判断 | 规则 recycle-bin（继承上层） | `C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV\vm_bundles\claudevm.bundle` | 回收站：你删掉的文件其实还在这里 |
| 20 | 9.58 GB | 需判断 | 规则 recycle-bin（继承上层） | `C:\$Recycle.Bin\S-1-5-21-988784579-10557099-473260007-1001\$RXPS4WV\vm_bundles` | 回收站：你删掉的文件其实还在这里 |
| 21 | 9.20 GB | 保留 | 规则 programdata-root | `C:\ProgramData` | 所有账户共用的软件数据（ProgramData，默认隐藏） |
| 22 | 8.18 GB | 需判断 | 规则 claude-desktop-vm（继承上层） | `C:\Users\king\AppData\Roaming\Claude\vm_bundles\claudevm.bundle` | Claude 桌面版下载的本地 Linux 虚拟机镜像 |
| 23 | 8.18 GB | 需判断 | 规则 claude-desktop-vm | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles` | Claude 桌面版下载的本地 Linux 虚拟机镜像 |
| 24 | 8.18 GB | 需判断 | 规则 claude-desktop-vm | `C:\Users\king\AppData\Roaming\Claude\vm_bundles` | Claude 桌面版下载的本地 Linux 虚拟机镜像 |
| 25 | 8.18 GB | 需判断 | 规则 claude-desktop-vm（继承上层） | `C:\Users\king\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle` | Claude 桌面版下载的本地 Linux 虚拟机镜像 |
| 26 | 7.60 GB | 需判断 | 规则 docker-local | `C:\Users\king\AppData\Local\Docker` | Docker Desktop 的本机数据（虚拟磁盘、日志、安装记录） |
| 27 | 7.57 GB | 保留 | 规则 docker-wsl | `C:\Users\king\AppData\Local\Docker\wsl` | Docker Desktop 的 WSL 虚拟磁盘文件夹 |
| 28 | 7.48 GB | 保留 | 规则 docker-data-vhdx | `C:\Users\king\AppData\Local\Docker\wsl\disk` | Docker Desktop 的数据盘：所有镜像、容器、数据卷都在这一个虚拟磁盘文件里 |
| 29 | 6.81 GB | 保留 | 规则 driverstore | `C:\Windows\System32\DriverStore` | 驱动仓库：装过的所有驱动安装包 |
| 30 | 6.81 GB | 保留 | 规则 driverstore（继承上层） | `C:\Windows\System32\DriverStore\FileRepository` | 驱动仓库：装过的所有驱动安装包 |
| 31 | 6.44 GB | 保留 | 规则 local-programs | `C:\Users\king\AppData\Local\Programs` | 只给你这个账户安装的软件（不需要管理员权限的那种） |
| 32 | 6.30 GB | 需判断 | 启发式 media-files | `C:\Users\king\AppData\ACLOS` | 一堆视频 / 图片 / 音频文件 |
| 33 | 6.30 GB | 需判断 | 启发式 media-files | `C:\Users\king\AppData\ACLOS\aclos-highlight` | 一堆视频 / 图片 / 音频文件 |
| 34 | 6.11 GB | 保留 | 规则 google-local | `C:\Users\king\AppData\Local\Google` | Google 家软件（主要是 Chrome）的本机数据 |
| 35 | 6.11 GB | 保留 | 规则 google-local | `C:\Users\king\AppData\Local\Google\Chrome` | Google 家软件（主要是 Chrome）的本机数据 |
| 36 | 6.11 GB | 保留 | 规则 chrome-profile | `C:\Users\king\AppData\Local\Google\Chrome\User Data` | Chrome 的用户资料：书签、密码、历史记录、扩展、登录状态 |
| 37 | 5.49 GB | 需判断 | 启发式 media-files | `C:\Users\king\AppData\ACLOS\aclos-highlight\wonderfulVideos690838325698893058` | 一堆视频 / 图片 / 音频文件 |
| 38 | 5.29 GB | 保留 | 规则 wsl-distro-vhdx | `C:\Users\king\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}` | WSL 里某个 Linux 发行版的整块硬盘（ext4.vhdx） |
| 39 | 5.29 GB | 保留 | 规则 wsl-root | `C:\Users\king\AppData\Local\wsl` | 新版 WSL 存放 Linux 发行版虚拟磁盘的文件夹 |
| 40 | 4.93 GB | 需判断 | 规则 feishu-larkshell | `C:\Users\king\AppData\Roaming\LarkShell` | 飞书的本地数据：各账号的聊天和文档缓存、内置浏览器数据、登录信息、更新包 |
| 41 | 4.79 GB | 保留 | 规则 nvidia-programdata | `C:\ProgramData\NVIDIA Corporation` | NVIDIA 驱动和软件的公共数据 |
| 42 | 4.59 GB | 保留 | 规则 program-files | `C:\Program Files (x86)` | 软件的安装目录（Program Files） |
| 43 | 4.59 GB | 需判断 | 规则 nvidia-app-data | `C:\ProgramData\NVIDIA Corporation\NVIDIA App` | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包） |
| 44 | 4.53 GB | 需判断 | 规则 nvidia-app-data（继承上层） | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework` | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包） |
| 45 | 4.53 GB | 需判断 | 规则 nvidia-app-updates | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts` | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包 |
| 46 | 4.51 GB | 保留 | 规则 office-program | `C:\Program Files\Microsoft Office` | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体 |
| 47 | 4.50 GB | 保留 | 规则 office-program（继承上层） | `C:\Program Files\Microsoft Office\root` | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体 |
| 48 | 4.20 GB | 可清理 | 规则 user-temp | `C:\Users\king\AppData\Local\Temp` | 你账户下的临时文件 |
| 49 | 3.98 GB | 需判断 | 规则 chrome-on-device-model | `C:\Users\king\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel` | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用） |
| 50 | 3.98 GB | 需判断 | 规则 chrome-on-device-model（继承上层） | `C:\Users\king\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel\2025.8.8.1141` | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用） |

#### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 34 |
| 规则继承上层 | 13 |
| 启发式 | 3 |
| 未知 | 0 |
| **规则 + 启发式覆盖率** | **100.0%**（50/50） |

#### 全盘按规则库归档（不含启发式）

| 档位 | 字节 |
|---|---:|
| 可清理 safe | 29.04 GB |
| 需判断 review | 60.89 GB |
| 保留 keep | 75.13 GB |
| 规则未覆盖 | 28.44 GB |

命中过的规则 93/179。safe 档最大的几项：
- `user-temp` 4.20 GB：你账户下的临时文件
- `uv-cache` 3.85 GB：uv 下载和解压过的 Python 包缓存
- `chromium-generic-cache` 3.56 GB：网页缓存（Chromium 内核程序通用格式）
- `chromium-generic-sw-cache` 3.38 GB：网页离线缓存（Service Worker 缓存）
- `chromium-generic-code-cache` 1.81 GB：脚本编译缓存（Chromium 内核程序通用）
- `gpu-shader-cache` 1.64 GB：显卡驱动的着色器缓存
- `vscode-family-cached` 1.48 GB：编辑器的缓存和日志（旧版本的编译缓存、扩展安装包副本、运行日志）
- `npm-cache` 1.42 GB：npm 下载过的包缓存

#### 未知项（下一批规则的待办）

（无）

### 附：前 200 大目录的统计与未知项（`--top 200`）

#### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 80 |
| 规则继承上层 | 92 |
| 启发式 | 19 |
| 未知 | 9 |
| **规则 + 启发式覆盖率** | **95.5%**（191/200） |

#### 未知项（下一批规则的待办）

- **#89 `C:\Users\king\AppData\Roaming\ACLOS`**（1.87 GB）
  - 最大的几个子项：TcVersions\（466.78 MB）、Fixs\（458.68 MB）、Cache\（319.46 MB）、esbrd\（252.24 MB）、estv\（135.07 MB）
  - 主要文件类型（按大小）：.zip 34%（9 个）、.exe 27%（6 个）、无扩展名 20%（379 个）、.dll 10%（71 个）、.log 3%（25 个）
  - 一共 2833 个文件、116 个文件夹
  - 最近一次修改在 2026-09-23
  - 在 AppData 下面：一般是某个软件自己存的数据（缓存、设置或下载的内容），不是你手动放的文件
  - 所在的软件文件夹叫「ACLOS」，但注册表“已安装的应用”里找不到对得上的软件（可能是绿色版、已卸载的残留，或者名字对不上）
- **#91 `C:\Users\king\.vscode`**（1.83 GB）
  - 最大的几个子项：extensions\（1.83 GB）、argv.json（4.00 KB）、settings.json（4.00 KB）、cli\（0 B）
  - 主要文件类型（按大小）：.exe 47%（24 个）、无扩展名 20%（243 个）、.js 16%（1248 个）、.dll 6%（178 个）、.pyi 3%（8739 个）
  - 一共 22785 个文件、2372 个文件夹
  - 最近一次修改在 2026-09-23
  - 在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的
- **#113 `C:\Users\king\.cache\codex-runtimes`**（1.35 GB）
  - 最大的几个子项：codex-primary-runtime\（1.34 GB）、codex-primary-runtime.previous-31450cd2-c150-4854-b46f-06c64e827b34\（18.32 MB）
  - 主要文件类型（按大小）：.dll 33%（400 个）、.exe 17%（80 个）、.js 7%（1388 个）、.pyc 6%（3702 个）、.node 6%（8 个）
  - 一共 21789 个文件、2035 个文件夹
  - 最近一次修改在 2026-09-13
  - 在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的
- **#115 `C:\Users\king\.cache\codex-runtimes\codex-primary-runtime`**（1.34 GB）
  - 最大的几个子项：dependencies\（1.33 GB）、plugins\（2.11 MB）、runtime.json（4.00 KB）
  - 主要文件类型（按大小）：.dll 33%（393 个）、.exe 17%（79 个）、.js 7%（1388 个）、.pyc 6%（3702 个）、.node 6%（8 个）
  - 一共 21768 个文件、2030 个文件夹
  - 最近一次修改在 2026-09-13
  - 在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的
- **#116 `C:\Users\king\.cache\codex-runtimes\codex-primary-runtime\dependencies`**（1.33 GB）
  - 最大的几个子项：native\（486.34 MB）、python\（450.42 MB）、node\（428.37 MB）、bin\（8.00 KB）
  - 主要文件类型（按大小）：.dll 33%（393 个）、.exe 17%（79 个）、.js 7%（1388 个）、.pyc 6%（3701 个）、.node 6%（8 个）
  - 一共 21564 个文件、1956 个文件夹
  - 最近一次修改在 2026-09-13
  - 在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的
- **#134 `C:\Users\king\AppData\Local\OpenAI`**（1.06 GB）
  - 最大的几个子项：Codex\（1.06 GB）、extension\（4.00 KB）
  - 主要文件类型（按大小）：.exe 93%（18 个）、.js 2%（2070 个）、.dll 2%（5 个）、.node 1%（21 个）、.br 1%（2 个）
  - 一共 4075 个文件、781 个文件夹
  - 最近一次修改在 2026-09-12
  - 在 AppData 下面：一般是某个软件自己存的数据（缓存、设置或下载的内容），不是你手动放的文件
  - 所在的软件文件夹叫「OpenAI」，但注册表“已安装的应用”里找不到对得上的软件（可能是绿色版、已卸载的残留，或者名字对不上）
- **#135 `C:\Users\king\AppData\Local\OpenAI\Codex`**（1.06 GB）
  - 最大的几个子项：bin\（824.55 MB）、runtimes\（259.43 MB）、chrome-native-hosts-v2.json（12.00 KB）、chrome-native-hosts.json（4.00 KB）
  - 主要文件类型（按大小）：.exe 93%（18 个）、.js 2%（2070 个）、.dll 2%（5 个）、.node 1%（21 个）、.br 1%（2 个）
  - 一共 4074 个文件、779 个文件夹
  - 最近一次修改在 2026-09-12
  - 在 AppData 下面：一般是某个软件自己存的数据（缓存、设置或下载的内容），不是你手动放的文件
  - 所在的软件文件夹叫「OpenAI」，但注册表“已安装的应用”里找不到对得上的软件（可能是绿色版、已卸载的残留，或者名字对不上）
- **#156 `C:\Users\king\.antigravity`**（916.34 MB）
  - 最大的几个子项：extensions\（916.34 MB）、argv.json（4.00 KB）
  - 主要文件类型（按大小）：.jar 23%（275 个）、.exe 15%（21 个）、无扩展名 11%（609 个）、.onnx 9%（2 个）、.js 8%（118 个）
  - 一共 5435 个文件、986 个文件夹
  - 最近一次修改在 2026-06-29
  - 在用户文件夹里，但不在 AppData 下：可能是你自己放的，也可能是某个工具建的
- **#167 `C:\Users\king\AppData\Local\OpenAI\Codex\bin`**（824.55 MB）
  - 最大的几个子项：7ac07f4ce733f89a\（373.40 MB）、codex.exe（234.43 MB）、5b9024f90663758b\（87.15 MB）、node.exe（87.15 MB）、34ab3e1324cc55b5\（12.95 MB）
  - 主要文件类型（按大小）：.exe 100%（13 个）、.json 0%（5 个）、.jsonl 0%（2 个）
  - 一共 20 个文件、7 个文件夹
  - 最近一次修改在 2026-09-12
  - 在 AppData 下面：一般是某个软件自己存的数据（缓存、设置或下载的内容），不是你手动放的文件
  - 所在的软件文件夹叫「OpenAI」，但注册表“已安装的应用”里找不到对得上的软件（可能是绿色版、已卸载的残留，或者名字对不上）

#### 未知项待办：下一批规则怎么写（我的判断，未核实的都标了）

| 路径 | 看到的线索 | 下一步 |
|---|---|---|
| `%APPDATA%\ACLOS`、`%USERPROFILE%\AppData\ACLOS` | Electron 程序（Cache / Code Cache / GPUCache 已被通配规则标绿），有 GameProxyCrashReports、TcVersions、Fixs，和 6 GB 的 wonderfulVideos 精彩时刻视频；注册表里没有登记 | 需要先确认是哪家的游戏加速/录制软件，再写规则：录像 → keep+migrate 或 review；TcVersions/Fixs → 待查 |
| `%USERPROFILE%\.vscode` | 只有 extensions（已有规则）+ argv.json/settings.json | 加容器规则（scope=self），很简单 |
| `%USERPROFILE%\.antigravity` | 同上，Antigravity 编辑器的扩展目录 | 同上 |
| `%USERPROFILE%\.cache\codex-runtimes` | dependencies 下有 native / python / node 三套运行时，runtime.json；另有 .previous-<GUID> 旧版本 | 看名字像 Codex 自带的运行时；需查 Codex 文档确认能否删、会不会自动重下 |
| `%LOCALAPPDATA%\OpenAI\Codex` | bin 里是 codex.exe、node.exe 和几个哈希命名的 exe 目录；runtimes | 看名字像 Codex 桌面/扩展的二进制；需确认它由谁安装、升级时旧版本是否残留 |

（另外，GUI 里 `AppData\Local` 的列表还能看到 kimi-desktop-updater（419 MB）、Pandoc（410 MB）等显示为“未知”的项，它们不在前 200 大目录里（第 200 名是 695 MB），没有计入上面的统计，也列为待办。）
