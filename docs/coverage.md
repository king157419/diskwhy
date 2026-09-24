覆盖率 = 在作者本机真实 C 盘上，DiskWhy 能把多少空间说清“这是什么”（靠规则库或启发式，不靠 AI、不算“还没认出来”）。第 1 轮评审后第一列改成**按字节的叶子覆盖率**，它最不容易被层层嵌套的文件夹抬高。

# 解释覆盖率（第 1 轮评审修复后，2026-09-23 夜）

日期：2026-09-23　机器：作者本机 Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
程序：分支 `phase-b`，`cargo build --release -p diskwhy-cli`（副本 `target\tmp\r1fix\diskwhy-cli-r1.exe`）
规则库：222 条内置；`%LOCALAPPDATA%\DiskWhy\rules` 里没有额外规则；注册表 Uninstall 快照 260 个已安装软件。路径里的用户名一律换成了 `<用户>`（`--md` 和 `--redact`）。

命令（只读，不联网）：

```
diskwhy-cli explain C:\ --top 50  --report live50.json  --md live50.md  --redact
diskwhy-cli explain C:\ --top 200 --report live200.json --md live200.md --redact
```

现扫 47.7 秒，卷已用 182.70 GB，解释 51 行用时 29 ms，全盘按规则库归档 368 ms。

## 结论

| 范围 | **叶子覆盖率（按字节）** | 规则 + 启发式（按个数） | 不算容器（按个数） | 未认出（按个数） |
|---|---:|---:|---:|---:|
| 前 50 大目录 | **58.9%**（98.54 GB / 167.45 GB） | 50 / 50 = 100% | 33 / 50 = 66% | 0 |
| 前 200 大目录 | **87.1%**（148.09 GB / 170.12 GB） | 199 / 200 = 99.5% | — | 1（`AppData\Local\OpenAI`，1.06 GB） |

- **叶子覆盖率**：每一行只算“没被表里更具体的行覆盖”的那部分字节（比如 `C:\Users\<用户>` 109.5 GB 减去表里它下面的 AppData 等行），看其中多少得到了**非容器**的说明。前 50 有 17 行是容器（`C:\`、`C:\Users`、用户目录、`AppData`、`AppData\Local`、`Program Files`、`ProgramData`、`Packages`、沙箱几层、`Docker`、`Google`、`wsl` 等），容器自己独占的约 69 GB 只有“里面混着各种东西，点进去看”这种说明，所以前 50 的叶子覆盖率只有 58.9%，**低于 PLAN 的 80%**。按个数算的 100% 是因为容器也算“命中”。前 200 行把这些容器里的大头都列出来了，叶子覆盖率到 87.1%。
- 按 PLAN 原来的口径（前 50 大目录里有规则或启发式说明的比例）仍是 100%；两个数都给，不藏。

## 逐行人工核对（前 50）

评审指出第一版“100% 里有说错的”，这一轮把前 50 每一行的说明都看了一遍，特别是商店应用沙箱那几行：

| # | 目录 | 现在的说明 | 对不对 |
|---:|---|---|---|
| 9 | `Packages\Claude_…` | claude-desktop-data：Claude 桌面版的本地数据 | 对（这一层确实是 Claude 的包文件夹，包含它的虚拟机镜像） |
| 10 | `Packages\Claude_…\LocalCache` | **store-app-localcache**：应用自己的缓存 + 它和它启动的程序写到 AppData 的东西 | 对（原来是继承 claude-desktop-data 的“登录信息、设置”，说错了） |
| 12 | `…\LocalCache\Roaming` | **store-app-sandbox-appdata**：商店应用沙箱，这个应用以及它启动的程序写到 AppData 的东西被 Windows 挪到了这里 | 对 |
| 13 | `…\LocalCache\Roaming\Claude` | claude-desktop-data（按原位置 `%APPDATA%\Claude` 找到的规则） | 对：这一块确实是 Claude 自己的漫游数据 |
| 15–16 | `…\Roaming\Claude\vm_bundles` 及下一层 | claude-desktop-vm | 对 |
| 20 | `…\LocalCache\Local`（7.56 GB） | **store-app-sandbox-appdata**（同 #12） | 对。**评审指出的那一行**：原来是“Claude 桌面版的登录信息、设置、界面缓存”，实际里面 7.01 GB 是另一轮会话的 cargo 编译目录，另有 pip、GitHub CLI、playwright 等 |
| 22、36、47 | `…\Local\DiskWhy-build\target` 及 `debug`、`debug\deps` | 启发式 rust-target：Rust 的编译产物（单独放在这里） | 对 |
| 23 | `…\Local\DiskWhy-build` | 启发式 rust-target：放 Rust 编译产物的文件夹（里面几乎全是 cargo 的 target） | 对（原来按包规则说明） |
| 25–26、32 | `AppData\ACLOS` 及下层 | aclos-game-recorder（review，写明“不确定，可能与《无畏契约》录像有关”） | 规则本身就标了不确定，说法诚实 |
| 35 | `AppData\Local\Temp` | user-temp，**但被护栏降成“需判断”**：里面有 19 个文档（共 29.8 MB），“其中约 5.14 GB 可以清理” | 对：这些 PDF 里有看起来是用户自己的“离线合订本”，不该一键删 |
| 其余 | Windows、System32、DriverStore、WinSxS、Installer、WindowsApps、Docker、WSL、Chrome、NVIDIA App、Office、`.claude` 等 | 对应的具体规则 | 逐行看过，没有发现说错的 |

## 全盘按规则库归档（现扫的完整树，含护栏）

| 档位 | 字节 | 和评审时（修复前）比 |
|---|---:|---|
| 可清理 safe | **23.83 GB** | 28.96 GB → 23.83 GB：Service Worker 的 CacheStorage / ScriptCache（3.47 GB）改成需判断；1.57 GB 名字像 Chromium 缓存、但内容不全是缓存格式的文件夹改记到需判断；Temp 里 30 MB 文档改记到需判断 |
| 需判断 review | 58.51 GB | |
| 保留 keep | 67.54 GB | |
| 规则没覆盖 | 20.74 GB | 沙箱里认不出的东西不再算到包规则上，所以这里会多一些 |

safe 里要用工具清（不能直接删文件夹）的只有 10 MB（本机没装 conda / pnpm）。


## 待办（按前 200 的结果）

前 200 里规则没命中的 18 行（17 行启发式 + 1 行未认出），合并嵌套后是 7 样东西：

| 目录 | 大小 | 现在怎么说明的 | 下一步 |
|---|---:|---|---|
| `Packages\Claude_…\LocalCache\Local\DiskWhy-build`（及其 `target`、`debug`、`debug\deps`、`debug\incremental`、`release`、`release\deps`，共 7 行） | 7.01 GB | 启发式 rust-target（review） | 已说对（本项目自己的编译目录），不需要规则 |
| `AppData\Local\Programs\GNU Octave`（及下面 5 行） | 2.77 GB | 启发式 registry-install-dir / registry-program-dir（程序本体，请正常卸载） | 已说对，优先级低 |
| `AppData\Local\Raspberry Pi` 和 `Raspberry Pi Imager`（2 行） | 1.24 GB | 启发式 registry-appdata（review）；里面唯一的 `cache` 由规则 `raspberry-pi-imager-cache` 说明为可清理 | 已说对。第 1 轮评审指出旧版 coverage.md 把它写成“要查、再写规则”——其实规则早在 5c4c7e0 就有了，写文档时没核对；本轮给它补了出处（rpi-imager issue #209）和旧版路径 `Raspberry Pi\Imager\cache`。可以再加一条两层容器规则 |
| `C:\Program Files\Google` | 0.93 GB | 启发式 registry-program-dir | 可加容器规则 |
| `C:\Program Files (x86)\Microsoft` | 0.82 GB | 启发式 registry-program-dir | 可加容器规则（里面是 Edge / EdgeWebView / EdgeUpdate，各自已有 keep 规则） |
| `C:\Program Files\LibreOffice` | 0.70 GB | 启发式 registry-install-dir | 可加 keep 规则（程序本体） |
| `AppData\Local\OpenAI`（主要是 `Codex\`） | 1.06 GB | **未认出**；线索小结“很可能是「OpenAI」/「Codex」相关的程序文件（.exe / .dll 占 95%）” | 还要查：OpenAI 官方文档里没找到 Codex 桌面程序装在这里的说明（没查证就不写规则） |

## 附：第一轮（修复前）的数字

修复前（B-3，规则 203 条）前 50：规则 47（精确 35，其中容器 14；继承 12）、启发式 3、未知 0，按个数 100%，不算容器 72%；全盘可清理 28.95 GB。那一轮的逐行核对只查出了沙箱里 `DiskWhy-build` 的子目录，漏了 #20 `LocalCache\Local` 这一层本身（产品评审第 1 条），这一轮补上了。

---

以下是 `--md` 自动生成的原始表格（前 50）。
## 前 50 大目录逐项解释

目标 `C:\`（输入：现扫）。扫描 48.5 秒，卷已用 182.71 GB；规则 222 条，注册表已安装软件 260 个；解释 51 行用时 30 ms。路径里的用户名已换成 `<用户>`。

| # | 目录 | 大小 | 档位 | 来源 | 一句话说明 |
|---:|---|---:|---|---|---|
| 本身 | `C:\` | 170.62 GB | 保留 | 规则 system-drive-root（容器） | 系统盘（C 盘）的根目录（属于：Windows） |
| 1 | `C:\Users` | 109.54 GB | 保留 | 规则 users-root（容器） | 所有 Windows 账户的个人文件夹都在这里（属于：Windows） |
| 2 | `C:\Users\<用户>` | 109.54 GB | 保留 | 规则 user-profile（容器） | 你这个账户的个人文件夹（属于：Windows） |
| 3 | `C:\Users\<用户>\AppData` | 95.03 GB | 保留 | 规则 appdata-root（容器） | 软件存放个人设置、缓存和数据的总文件夹（默认隐藏）（属于：Windows） |
| 4 | `C:\Users\<用户>\AppData\Local` | 68.48 GB | 保留 | 规则 appdata-local（容器） | 只属于这台电脑的软件数据（AppData\Local）（属于：Windows） |
| 5 | `C:\Windows` | 25.75 GB | 保留 | 规则 windows-dir | Windows 系统本身（属于：Windows） |
| 6 | `C:\Program Files` | 22.97 GB | 保留 | 规则 program-files（容器） | 软件的安装目录（Program Files）（属于：Windows） |
| 7 | `C:\Users\<用户>\AppData\Roaming` | 20.12 GB | 保留 | 规则 appdata-roaming（容器） | 软件的设置和个人数据（AppData\Roaming）（属于：Windows） |
| 8 | `C:\Users\<用户>\AppData\Local\Packages` | 19.04 GB | 保留 | 规则 store-apps-data（容器） | 商店应用（MSIX 包）的数据，每个应用一个文件夹（属于：Microsoft Store 应用） |
| 9 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc` | 17.66 GB | 需判断 | 规则 claude-desktop-data | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 10 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache` | 17.66 GB | 需判断 | 规则 store-app-localcache（容器） | 商店应用的 LocalCache：应用自己的缓存，加上它（和它启动的程序）写到 AppData 的东西（属于：claude（Microsoft Store 应用）） |
| 11 | `C:\Windows\System32` | 11.02 GB | 保留 | 规则 system32 | Windows 的核心程序和系统库（SysWOW64 是给 32 位程序用的那一套）（属于：Windows） |
| 12 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming` | 10.09 GB | 需判断 | 规则 store-app-sandbox-appdata（容器） | 商店应用沙箱：这个应用以及它启动的程序写到 AppData 的东西，被 Windows 挪到了这里（属于：claude（Microsoft Store 应用（沙箱））） |
| 13 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` | 10.06 GB | 需判断 | 规则 claude-desktop-data | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 14 | `C:\ProgramData` | 9.19 GB | 保留 | 规则 programdata-root（容器） | 所有账户共用的软件数据（ProgramData，默认隐藏）（属于：Windows） |
| 15 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles` | 8.18 GB | 需判断 | 规则 claude-desktop-vm | Claude 桌面版下载的本地 Linux 虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 16 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle` | 8.18 GB | 需判断 | 规则 claude-desktop-vm（继承上层） | Claude 桌面版下载的本地 Linux 虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 17 | `C:\Program Files\WindowsApps` | 7.65 GB | 保留 | 规则 windows-apps | 商店应用（Microsoft Store 应用）的程序本体（属于：Microsoft Store） |
| 18 | `C:\Users\<用户>\AppData\Local\Docker` | 7.60 GB | 需判断 | 规则 docker-local（容器） | Docker Desktop 的本机数据（虚拟磁盘、日志、安装记录）（属于：Docker Desktop） |
| 19 | `C:\Users\<用户>\AppData\Local\Docker\wsl` | 7.57 GB | 保留 | 规则 docker-wsl | Docker Desktop 的 WSL 虚拟磁盘文件夹（属于：Docker Desktop） |
| 20 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local` | 7.56 GB | 需判断 | 规则 store-app-sandbox-appdata（容器） | 商店应用沙箱：这个应用以及它启动的程序写到 AppData 的东西，被 Windows 挪到了这里（属于：claude（Microsoft Store 应用（沙箱））） |
| 21 | `C:\Users\<用户>\AppData\Local\Docker\wsl\disk` | 7.48 GB | 保留 | 规则 docker-data-vhdx | Docker Desktop 的数据盘：所有镜像、容器、数据卷都在这一个虚拟磁盘文件里（属于：Docker Desktop） |
| 22 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target` | 7.01 GB | 需判断 | 启发式 rust-target | Rust 的编译产物目录（cargo 的 target，单独放在这里） |
| 23 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build` | 7.01 GB | 需判断 | 启发式 rust-target | 放 Rust 编译产物的文件夹（里面几乎全是 cargo 的 target） |
| 24 | `C:\Users\<用户>\AppData\Local\Programs` | 6.44 GB | 保留 | 规则 local-programs（容器） | 只给你这个账户安装的软件（不需要管理员权限的那种）（属于：Windows） |
| 25 | `C:\Users\<用户>\AppData\ACLOS\aclos-highlight` | 6.30 GB | 需判断 | 规则 aclos-game-recorder（继承上层） | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 26 | `C:\Users\<用户>\AppData\ACLOS` | 6.30 GB | 需判断 | 规则 aclos-game-recorder | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 27 | `C:\Users\<用户>\AppData\Local\Google` | 6.12 GB | 保留 | 规则 google-local（容器） | Google 家软件（主要是 Chrome）的本机数据（属于：Google） |
| 28 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data` | 6.12 GB | 保留 | 规则 chrome-profile | Chrome 的用户资料：书签、密码、历史记录、扩展、登录状态（属于：Google Chrome） |
| 29 | `C:\Users\<用户>\AppData\Local\Google\Chrome` | 6.12 GB | 保留 | 规则 google-local（容器） | Google 家软件（主要是 Chrome）的本机数据（属于：Google） |
| 30 | `C:\Windows\System32\DriverStore` | 6.07 GB | 保留 | 规则 driverstore | 驱动仓库：装过的所有驱动安装包（属于：Windows） |
| 31 | `C:\Windows\System32\DriverStore\FileRepository` | 6.06 GB | 保留 | 规则 driverstore（继承上层） | 驱动仓库：装过的所有驱动安装包（属于：Windows） |
| 32 | `C:\Users\<用户>\AppData\ACLOS\aclos-highlight\wonderfulVideos690838325698893058` | 5.49 GB | 需判断 | 规则 aclos-game-recorder（继承上层） | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 33 | `C:\Users\<用户>\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}` | 5.29 GB | 保留 | 规则 wsl-distro-vhdx | WSL 里某个 Linux 发行版的整块硬盘（ext4.vhdx）（属于：WSL（适用于 Linux 的 Windows 子系统）） |
| 34 | `C:\Users\<用户>\AppData\Local\wsl` | 5.29 GB | 保留 | 规则 wsl-root（容器） | 新版 WSL 存放 Linux 发行版虚拟磁盘的文件夹（属于：WSL） |
| 35 | `C:\Users\<用户>\AppData\Local\Temp` | 5.16 GB | 需判断 | 规则 user-temp | 你账户下的临时文件（属于：Windows） |
| 36 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug` | 5.13 GB | 需判断 | 启发式 rust-target | Rust 编译产物的一部分（cargo 的 target，单独放在这里） |
| 37 | `C:\Users\<用户>\AppData\Roaming\LarkShell` | 4.93 GB | 需判断 | 规则 feishu-larkshell | 飞书的本地数据：各账号的聊天和文档缓存、内置浏览器数据、登录信息、更新包（属于：飞书） |
| 38 | `C:\ProgramData\NVIDIA Corporation` | 4.79 GB | 保留 | 规则 nvidia-programdata（容器） | NVIDIA 驱动和软件的公共数据（属于：NVIDIA） |
| 39 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App` | 4.59 GB | 需判断 | 规则 nvidia-app-data | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包）（属于：NVIDIA App） |
| 40 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework` | 4.53 GB | 需判断 | 规则 nvidia-app-data（继承上层） | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包）（属于：NVIDIA App） |
| 41 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts` | 4.53 GB | 需判断 | 规则 nvidia-app-updates | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包（属于：NVIDIA App） |
| 42 | `C:\Program Files\Microsoft Office` | 4.51 GB | 保留 | 规则 office-program | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体（属于：Microsoft Office） |
| 43 | `C:\Program Files\Microsoft Office\root` | 4.50 GB | 保留 | 规则 office-program（继承上层） | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体（属于：Microsoft Office） |
| 44 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel` | 3.98 GB | 需判断 | 规则 chrome-on-device-model | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用）（属于：Google Chrome） |
| 45 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel\2025.8.8.1141` | 3.98 GB | 需判断 | 规则 chrome-on-device-model（继承上层） | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用）（属于：Google Chrome） |
| 46 | `C:\Windows\Installer` | 3.81 GB | 保留 | 规则 windows-installer-cache | 已装软件的安装包缓存（.msi / .msp）（属于：Windows Installer） |
| 47 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug\deps` | 3.78 GB | 需判断 | 启发式 rust-target | Rust 编译产物的一部分（cargo 的 target，单独放在这里） |
| 48 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts\grd` | 3.68 GB | 需判断 | 规则 nvidia-app-updates（继承上层） | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包（属于：NVIDIA App） |
| 49 | `C:\Windows\WinSxS` | 3.65 GB | 保留 | 规则 winsxs | Windows 组件仓库：系统组件和更新的所有版本（属于：Windows） |
| 50 | `C:\Users\<用户>\.claude` | 3.59 GB | 需判断 | 规则 claude-code-data | Claude Code 的配置、会话记录和插件（属于：Claude Code（Anthropic 命令行工具）） |

### 结论（50 个目录）

| 叶子覆盖率（按字节） | 规则 + 启发式（按个数） | 不算容器（按个数） | 未知（按个数） |
|---:|---:|---:|---:|
| **58.9%**（98.54 GB / 167.45 GB） | 100.0% | 66.0% | 0.0% |

叶子覆盖率：每一行只算“没被表里更具体的行覆盖”的那部分字节，看其中多少得到了非容器的说明（容器规则只说“这是用户文件夹”这类话，不算）。这是最不容易被层层嵌套抬高的口径。

### 命中率（50 个目录）

| 来源 | 个数 | 占比 |
|---|---:|---:|
| 规则精确命中 | 38 | 76.0% |
| 　其中“容器”规则（只说明文件夹本身，如 `C:\Users`） | 17 | 34.0% |
| 规则继承上层 | 8 | 16.0% |
| **规则合计** | **46** | **92.0%** |
| 启发式 | 4 | 8.0% |
| 未知 | 0 | 0.0% |
| **规则 + 启发式** | **50** | **100.0%** |

### 按规则库归档（不含启发式）

| 档位 | 字节 |
|---|---:|
| 可清理 safe | 23.83 GB |
| 需判断 review | 58.51 GB |
| 保留 keep | 67.54 GB |
| 规则未覆盖 | 20.74 GB |

命中过的规则 101/222。safe 里要用工具清（不能直接删文件夹）的有 10.28 MB；护栏从 safe / review 改记到 review / keep 的（缓存目录里像用户数据的部分、不合缓存格式的通配命中）有 1.60 GB。
- 护栏改记 名字像 Chromium 缓存、但内容不全是缓存格式的文件夹：1.57 GB，最大一处 `C:\Users\<用户>\AppData\Roaming\ACLOS\Cache`（319.46 MB）
- 护栏改记 缓存 / 临时文件夹里的文档：30.17 MB，最大一处 `C:\Users\<用户>\AppData\Local\Temp\claude\D--desk-claudex\f9edb0bb-57b2-4b14-a86a-c7c6fab75791\scratchpad\offb-r2\print-scrolled.pdf`（11.37 MB）

### 规则没命中的前 10 个目录（下一批规则的待办）

- **#22 `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target`**（7.01 GB）：启发式 rust-target — Rust 的编译产物目录（cargo 的 target，单独放在这里）
  - C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target 里有 CACHEDIR.TAG 和 .rustc_info.json（cargo 编译目录的标记），旁边没有 Cargo.toml，看不出是哪个项目
- **#23 `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build`**（7.01 GB）：启发式 rust-target — 放 Rust 编译产物的文件夹（里面几乎全是 cargo 的 target）
  - 子文件夹 target 里有 CACHEDIR.TAG 和 .rustc_info.json（cargo 编译目录的标记），占了这里 100%
- **#36 `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug`**（5.13 GB）：启发式 rust-target — Rust 编译产物的一部分（cargo 的 target，单独放在这里）
  - C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target 里有 CACHEDIR.TAG 和 .rustc_info.json（cargo 编译目录的标记），旁边没有 Cargo.toml，看不出是哪个项目
- **#47 `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug\deps`**（3.78 GB）：启发式 rust-target — Rust 编译产物的一部分（cargo 的 target，单独放在这里）
  - C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target 里有 CACHEDIR.TAG 和 .rustc_info.json（cargo 编译目录的标记），旁边没有 Cargo.toml，看不出是哪个项目
