覆盖率 = 在作者本机真实 C 盘上，前 N 大目录里有多少个能被 DiskWhy 说清“这是什么”（靠规则库或启发式，不靠 AI、不算“我也不确定”）。

# 解释覆盖率（阶段 B-3，2026-09-23 晚）

日期：2026-09-23　机器：作者本机 Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
程序：分支 `phase-b`（提交 5af4fe7 之后的代码），`cargo build --release -p diskwhy-cli`
规则库：203 条内置（6 个分组）+ 本机 `%LOCALAPPDATA%\DiskWhy\rules` 里没有额外规则被加载；注册表 Uninstall 快照 260 个已安装软件。
路径里的用户名一律换成了 `<用户>`。

命令（都是只读的）：

```
diskwhy-cli scan C:\ --top 50 --json target\tmp\scan.json --snapshot target\tmp\c.dwsnap --quiet
diskwhy-cli explain --from target\tmp\scan.json --top 50 --json explain50.json --report report50.json --md cov50.md
diskwhy-cli explain --from target\tmp\c.dwsnap --top 200 --report report200.json --md cov200.md     （找待办用）
```

扫描 60.1 秒（比平时的 41 秒慢，因为同时另一轮会话在同一块盘上编译）；`scan.json` 24 MB，里面 24,186 个目录节点**每个都带了解释摘要**（档位 / 属于谁 / 来源 / 一句话），算摘要用了 224 ms。`explain --from scan.json` 解释 51 行用时 219 ms，其中 2 个文件夹在 JSON 里被截断、回磁盘补读了一遍（215 ms）。用脚本逐行核对：`explain` 的 51 行和 `scan.json` 里对应节点的摘要**51 行全部一致**。

## 结论

| 范围 | 规则（精确 + 继承上层） | 其中“容器”规则 | 启发式 | 未知 | **规则 + 启发式** |
|---|---:|---:|---:|---:|---:|
| **前 50 大目录**（PLAN 的成功标准，目标 ≥ 80%） | 47 = 94.0%（35 + 12） | 14 | 3 = 6.0% | 0 = 0.0% | **50 / 50 = 100.0%** |
| 前 200 大目录（找待办用） | 183 = 91.5%（82 + 101） | 22 | 16 = 8.0% | 1 = 0.5% | **199 / 200 = 99.5%** |

达到目标，所以本轮**没有为了凑数加规则**（任务说的“达不到就补规则再跑，最多两轮”没有触发）。第二轮扫描是因为第一轮发现了说错的地方（见下面第 3 条），修完代码后重扫验证。

### 这些数字要怎么看

1. **“容器”也算命中**：前 50 里有 14 个是 `scope = "self"` 的容器规则（`C:\Users`、`C:\Users\<用户>`、`AppData`、`AppData\Local`、`Program Files`、`ProgramData`、`Packages`、`Docker`、`Google` 等）。它们回答了“这是什么”，但回答是“里面混着各种东西，点进去看”。**不算容器的话，前 50 是 36 / 50 = 72%**，低于 80%；这 14 个容器都是更具体的子目录的上层，子目录本身在表里都有具体说明。
2. **前 50 是嵌套的**：`C:\Users` ⊃ `C:\Users\<用户>` ⊃ `AppData` ⊃ …，50 行不是 50 块互不相交的空间。
3. **第一轮发现并修掉的错**：第一轮（修改前）前 50 同样是 100%，但其中 4 行（`Packages\Claude_…\LocalCache\Local\DiskWhy-build`、它下面的 `target`、`target\debug`、`target\debug\deps`，是同一块约 7 GB）被继承规则 `claude-desktop-data` 说成“Claude 桌面版的登录信息、设置、界面缓存”——**说错了**。实际上这是 Windows 对商店应用的沙箱重定向：Claude 桌面版（以及它启动的程序，包括跑这个任务的会话）在 `AppData\Local` 下**新建**的文件夹，会被写到 `Packages\<包名>\LocalCache\Local\` 下（learn.microsoft.com《Understanding how packaged desktop apps run on Windows》的 AppData 一节）。那个 7 GB 其实是另一轮会话用 `CARGO_TARGET_DIR` 放的 Rust 编译目录。修法：
   - 新增“沙箱重定向”：路径落在 `Packages\<包>\LocalCache\{Local,Roaming,LocalLow}\…` 时，先按应用以为的原位置（`%LOCALAPPDATA%\…` / `%APPDATA%\…`）找规则、找上层规则、跑启发式；都认不出，才退回包自己的规则，并加一句“这里不一定都是这个应用自己的数据”。
   - `rust-target` 启发式多认一种：目录里同时有 `CACHEDIR.TAG` 和 `.rustc_info.json`（cargo 编译目录的标记，旁边没有 Cargo.toml 也算）。只查这两个文件在不在，不读内容。
   第二轮这 3 行变成“启发式 rust-target：Rust 的编译产物目录（cargo 的 target，单独放在这里）”，`DiskWhy-build` 本身仍按包规则说明但带提醒。这也是本轮启发式从 0 变成 3 的原因。
4. **“规则按原位置说明”对沙箱里的其他东西也生效**：本机 `LocalCache\Local` 里还有 `pip`、`GitHub CLI`、`ms-playwright-mcp` 等（都是从 Claude 桌面版里启动的程序写的），现在 `LocalCache\Local\pip\cache`（本机 141 MB）会按 `pip-cache` 规则说明为“可清理”（已用 `diskwhy-cli explain <那个路径>` 在本机实测）。它们都不在前 50 里。
5. **启发式和规则都只看名字、大小、时间和“某个文件在不在”**，没有打开任何文件读内容；没有联网（`--llm` 没开）。
6. **全盘按规则库归档**（不含启发式，按完整快照算，122 ms）：可清理 28.95 GB｜需判断 59.76 GB｜保留 67.53 GB｜规则未覆盖 13.95 GB｜命中过的规则 94 / 203。`explain --from scan.json` 输出里的归档数（7.75 / 69.66 / 70.87 / 21.90 GB）是按截断过的 JSON 树算的，偏粗，以快照这组为准。

## 前 50 大目录逐项解释

目标 `C:\`（输入：scan --json 输出（目录树截断过））。扫描 60.1 秒，卷已用 182.28 GB；规则 203 条，注册表已安装软件 260 个；解释 51 行用时 219 ms（其中回磁盘补读 2 个被截断的文件夹，215 ms）。路径里的用户名已换成 `<用户>`。

| # | 目录 | 大小 | 档位 | 来源 | 一句话说明 |
|---:|---|---:|---|---|---|
| 本身 | `C:\` | 170.20 GB | 保留 | 规则 system-drive-root（容器） | 系统盘（C 盘）的根目录（属于：Windows） |
| 1 | `C:\Users` | 109.10 GB | 保留 | 规则 users-root（容器） | 所有 Windows 账户的个人文件夹都在这里（属于：Windows） |
| 2 | `C:\Users\<用户>` | 109.10 GB | 保留 | 规则 user-profile（容器） | 你这个账户的个人文件夹（属于：Windows） |
| 3 | `C:\Users\<用户>\AppData` | 94.66 GB | 保留 | 规则 appdata-root（容器） | 软件存放个人设置、缓存和数据的总文件夹（默认隐藏）（属于：Windows） |
| 4 | `C:\Users\<用户>\AppData\Local` | 68.13 GB | 保留 | 规则 appdata-local（容器） | 只属于这台电脑的软件数据（AppData\Local）（属于：Windows） |
| 5 | `C:\Windows` | 25.75 GB | 保留 | 规则 windows-dir | Windows 系统本身（属于：Windows） |
| 6 | `C:\Program Files` | 22.97 GB | 保留 | 规则 program-files（容器） | 软件的安装目录（Program Files）（属于：Windows） |
| 7 | `C:\Users\<用户>\AppData\Roaming` | 20.10 GB | 保留 | 规则 appdata-roaming（容器） | 软件的设置和个人数据（AppData\Roaming）（属于：Windows） |
| 8 | `C:\Users\<用户>\AppData\Local\Packages` | 19.04 GB | 保留 | 规则 store-apps-data（容器） | 商店应用（MSIX 包）的数据，每个应用一个文件夹（属于：Microsoft Store 应用） |
| 9 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc` | 17.66 GB | 需判断 | 规则 claude-desktop-data | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 10 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache` | 17.66 GB | 需判断 | 规则 claude-desktop-data（继承上层） | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 11 | `C:\Windows\System32` | 11.02 GB | 保留 | 规则 system32 | Windows 的核心程序和系统库（SysWOW64 是给 32 位程序用的那一套）（属于：Windows） |
| 12 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming` | 10.09 GB | 需判断 | 规则 claude-desktop-data（继承上层） | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 13 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` | 10.06 GB | 需判断 | 规则 claude-desktop-data | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 14 | `C:\ProgramData` | 9.20 GB | 保留 | 规则 programdata-root（容器） | 所有账户共用的软件数据（ProgramData，默认隐藏）（属于：Windows） |
| 15 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles` | 8.18 GB | 需判断 | 规则 claude-desktop-vm | Claude 桌面版下载的本地 Linux 虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 16 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\vm_bundles\claudevm.bundle` | 8.18 GB | 需判断 | 规则 claude-desktop-vm（继承上层） | Claude 桌面版下载的本地 Linux 虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 17 | `C:\Program Files\WindowsApps` | 7.65 GB | 保留 | 规则 windows-apps | 商店应用（Microsoft Store 应用）的程序本体（属于：Microsoft Store） |
| 18 | `C:\Users\<用户>\AppData\Local\Docker` | 7.60 GB | 需判断 | 规则 docker-local（容器） | Docker Desktop 的本机数据（虚拟磁盘、日志、安装记录）（属于：Docker Desktop） |
| 19 | `C:\Users\<用户>\AppData\Local\Docker\wsl` | 7.57 GB | 保留 | 规则 docker-wsl | Docker Desktop 的 WSL 虚拟磁盘文件夹（属于：Docker Desktop） |
| 20 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local` | 7.56 GB | 需判断 | 规则 claude-desktop-data（继承上层） | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 21 | `C:\Users\<用户>\AppData\Local\Docker\wsl\disk` | 7.48 GB | 保留 | 规则 docker-data-vhdx | Docker Desktop 的数据盘：所有镜像、容器、数据卷都在这一个虚拟磁盘文件里（属于：Docker Desktop） |
| 22 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target` | 7.01 GB | 需判断 | 启发式 rust-target | Rust 的编译产物目录（cargo 的 target，单独放在这里） |
| 23 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build` | 7.01 GB | 需判断 | 规则 claude-desktop-data（继承上层） | Claude 桌面版的本地数据：登录信息、设置、界面缓存，以及虚拟机镜像（属于：Claude 桌面版（Anthropic）） |
| 24 | `C:\Users\<用户>\AppData\Local\Programs` | 6.44 GB | 保留 | 规则 local-programs（容器） | 只给你这个账户安装的软件（不需要管理员权限的那种）（属于：Windows） |
| 25 | `C:\Users\<用户>\AppData\ACLOS\aclos-highlight` | 6.30 GB | 需判断 | 规则 aclos-game-recorder（继承上层） | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 26 | `C:\Users\<用户>\AppData\ACLOS` | 6.30 GB | 需判断 | 规则 aclos-game-recorder | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 27 | `C:\Users\<用户>\AppData\Local\Google` | 6.11 GB | 保留 | 规则 google-local（容器） | Google 家软件（主要是 Chrome）的本机数据（属于：Google） |
| 28 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data` | 6.11 GB | 保留 | 规则 chrome-profile | Chrome 的用户资料：书签、密码、历史记录、扩展、登录状态（属于：Google Chrome） |
| 29 | `C:\Users\<用户>\AppData\Local\Google\Chrome` | 6.11 GB | 保留 | 规则 google-local（容器） | Google 家软件（主要是 Chrome）的本机数据（属于：Google） |
| 30 | `C:\Windows\System32\DriverStore` | 6.07 GB | 保留 | 规则 driverstore | 驱动仓库：装过的所有驱动安装包（属于：Windows） |
| 31 | `C:\Windows\System32\DriverStore\FileRepository` | 6.06 GB | 保留 | 规则 driverstore（继承上层） | 驱动仓库：装过的所有驱动安装包（属于：Windows） |
| 32 | `C:\Users\<用户>\AppData\ACLOS\aclos-highlight\wonderfulVideos690838325698893058` | 5.49 GB | 需判断 | 规则 aclos-game-recorder（继承上层） | 某个游戏相关程序的数据，以及它录下来的游戏视频（属于：不确定（可能与腾讯《无畏契约》的“无畏时刻”录像有关）） |
| 33 | `C:\Users\<用户>\AppData\Local\wsl\{d1f9e0f8-2451-4607-a4db-2cfc50dc641c}` | 5.29 GB | 保留 | 规则 wsl-distro-vhdx | WSL 里某个 Linux 发行版的整块硬盘（ext4.vhdx）（属于：WSL（适用于 Linux 的 Windows 子系统）） |
| 34 | `C:\Users\<用户>\AppData\Local\wsl` | 5.29 GB | 保留 | 规则 wsl-root（容器） | 新版 WSL 存放 Linux 发行版虚拟磁盘的文件夹（属于：WSL） |
| 35 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug` | 5.13 GB | 需判断 | 启发式 rust-target | Rust 编译产物的一部分（cargo 的 target，单独放在这里） |
| 36 | `C:\Users\<用户>\AppData\Roaming\LarkShell` | 4.93 GB | 需判断 | 规则 feishu-larkshell | 飞书的本地数据：各账号的聊天和文档缓存、内置浏览器数据、登录信息、更新包（属于：飞书） |
| 37 | `C:\Users\<用户>\AppData\Local\Temp` | 4.81 GB | 可清理 | 规则 user-temp | 你账户下的临时文件（属于：Windows） |
| 38 | `C:\ProgramData\NVIDIA Corporation` | 4.79 GB | 保留 | 规则 nvidia-programdata（容器） | NVIDIA 驱动和软件的公共数据（属于：NVIDIA） |
| 39 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App` | 4.59 GB | 需判断 | 规则 nvidia-app-data | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包）（属于：NVIDIA App） |
| 40 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework` | 4.53 GB | 需判断 | 规则 nvidia-app-data（继承上层） | NVIDIA App 的公共数据（设置、日志，大头通常是下载的更新包）（属于：NVIDIA App） |
| 41 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts` | 4.53 GB | 需判断 | 规则 nvidia-app-updates | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包（属于：NVIDIA App） |
| 42 | `C:\Program Files\Microsoft Office` | 4.51 GB | 保留 | 规则 office-program | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体（属于：Microsoft Office） |
| 43 | `C:\Program Files\Microsoft Office\root` | 4.50 GB | 保留 | 规则 office-program（继承上层） | Microsoft Office（Word、Excel、PowerPoint 等）的程序本体（属于：Microsoft Office） |
| 44 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel\2025.8.8.1141` | 3.98 GB | 需判断 | 规则 chrome-on-device-model（继承上层） | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用）（属于：Google Chrome） |
| 45 | `C:\Users\<用户>\AppData\Local\Google\Chrome\User Data\OptGuideOnDeviceModel` | 3.98 GB | 需判断 | 规则 chrome-on-device-model | Chrome 下载到本机的 AI 模型（给浏览器内置的 AI 功能用）（属于：Google Chrome） |
| 46 | `C:\Windows\Installer` | 3.81 GB | 保留 | 规则 windows-installer-cache | 已装软件的安装包缓存（.msi / .msp）（属于：Windows Installer） |
| 47 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target\debug\deps` | 3.78 GB | 需判断 | 启发式 rust-target | Rust 编译产物的一部分（cargo 的 target，单独放在这里） |
| 48 | `C:\ProgramData\NVIDIA Corporation\NVIDIA App\UpdateFramework\ota-artifacts\grd` | 3.68 GB | 需判断 | 规则 nvidia-app-updates（继承上层） | NVIDIA App（或旧的 GeForce Experience）下载的驱动和程序更新包（属于：NVIDIA App） |
| 49 | `C:\Windows\WinSxS` | 3.65 GB | 保留 | 规则 winsxs | Windows 组件仓库：系统组件和更新的所有版本（属于：Windows） |
| 50 | `C:\Users\<用户>\.claude` | 3.53 GB | 需判断 | 规则 claude-code-data | Claude Code 的配置、会话记录和插件（属于：Claude Code（Anthropic 命令行工具）） |

### 命中率（50 个目录）

| 来源 | 个数 | 占比 |
|---|---:|---:|
| 规则精确命中 | 35 | 70.0% |
| 　其中“容器”规则（只说明文件夹本身，如 `C:\Users`） | 14 | 28.0% |
| 规则继承上层 | 12 | 24.0% |
| **规则合计** | **47** | **94.0%** |
| 启发式 | 3 | 6.0% |
| 未知 | 0 | 0.0% |
| **规则 + 启发式** | **50** | **100.0%** |

### 按规则库归档（不含启发式；JSON 树截断过，数字偏粗）

| 档位 | 字节 |
|---|---:|
| 可清理 safe | 7.75 GB |
| 需判断 review | 69.66 GB |
| 保留 keep | 70.87 GB |
| 规则未覆盖 | 21.90 GB |

命中过的规则 75/203。


## 未命中的前 10 个目录（下一批规则的待办）

“未命中”= 没有规则说明（靠启发式、AI 或者“我也不确定”）。前 50 里只有 3 个（都是同一个 cargo 编译目录），所以按 `--top 200` 的结果、按大小取前 10；嵌套的同一样东西合并成一条，合并后 7 条正好覆盖前 200 里全部 17 个未命中的目录（16 个启发式 + 1 个未知）。

| # | 目录 | 大小 | 现在怎么说明的 | 下一步 |
|---:|---|---:|---|---|
| 22 | `C:\Users\<用户>\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\DiskWhy-build\target`（含 #35 `debug`、#47 `debug\deps`、#83 `release`、#92 `release\deps`、#155 `debug\incremental`） | 7.01 GB | 启发式 rust-target（review，删了只是要重新编译） | 不需要规则：这是本项目自己的编译目录（另一轮会话用 `CARGO_TARGET_DIR` 放在这里）。可以考虑启发式在沙箱里时把“单独放在这里”说得更具体。它的上层 #23 `DiskWhy-build` 算规则命中（按包规则说明 + “不一定是这个应用的数据”提醒），更好的说法是“里面只有一个 cargo 编译目录”，可以加一条启发式 |
| 63 | `C:\Users\<用户>\AppData\Local\Programs\GNU Octave\Octave-11.3.0`（含 #64 上层、#76 `mingw64`、#175 `lib`、#183 `bin`） | 2.77 GB | 启发式 registry-install-dir（注册表里登记的安装目录，想腾空间请正常卸载） | 已经说得对。可加一条 keep 规则 `%LOCALAPPDATA%\Programs\GNU Octave`，但启发式已够用，优先级低 |
| 122 | `C:\Users\<用户>\AppData\Local\Raspberry Pi\Raspberry Pi Imager`（含 #123 上层） | 1.24 GB | 启发式 registry-appdata（名字和已安装的 Raspberry Pi Imager 对得上） | **要查**：1.24 GB 多半是它下载缓存的系统镜像。需要查 Raspberry Pi Imager 官方说明确认缓存位置和删了的后果，再写规则（很可能是 review + 可删） |
| 136 | `C:\Users\<用户>\AppData\Local\OpenAI`（主要是 `Codex\`） | 1.06 GB | **未知**（.exe 占 93%，注册表里没有对得上的软件） | **要查**：看起来是 OpenAI Codex 的程序文件（CLI / 扩展自带的可执行文件）。需要查官方文档确认安装位置后写规则；在确认前保持“我也不确定” |
| 154 | `C:\Program Files\Google` | 948 MB | 启发式 registry-program-dir（程序文件夹，发布者 Google） | 可加容器规则（Chrome 程序本体在 `Program Files\Google\Chrome\Application`，已有规则覆盖到子目录的话只差这一层） |
| 167 | `C:\Program Files (x86)\Microsoft` | 843 MB | 启发式 registry-program-dir（发布者 Microsoft） | 可加容器规则：里面通常是 Edge / EdgeWebView / EdgeUpdate，各自是程序本体（keep） |
| 193 | `C:\Program Files\LibreOffice` | 714 MB | 启发式 registry-install-dir | 可加 keep 规则（程序本体，卸载请走系统设置） |

说明：上表的“下一步”是我的判断；标“要查”的两条**没有查证**，按项目规则（先查官方文档再写）留给下一批。

## 附：第一轮（修改前）的结果

第一轮用同一套命令（扫描 64.4 秒）：前 50 规则 50（精确 34，其中容器 14；继承上层 16），启发式 0，未知 0，覆盖率 100%。数字一样，但 4 行说错了（见上面“这些数字要怎么看”第 3 条）。所以只看覆盖率会高估质量——这一轮顺手做了逐行人工核对，下一轮也应该这样做。
