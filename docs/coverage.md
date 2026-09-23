# 解释覆盖率（阶段 B）

日期：2026-09-23　机器：作者本机 Windows 11 Pro 10.0.26200，C 盘 NTFS 199 GB，**普通用户权限（未提权）**
命令：`target\release\diskwhy-cli.exe explain C:\ --top 50 --quiet --md ... --json ...`（另跑了一次 `--top 200` 找待办）
规则库：179 条（6 个分组），注册表 Uninstall 快照 260 个已安装软件。

## 结论

| 范围 | 规则（精确 + 继承上层） | 启发式 | 未知 | 规则 + 启发式覆盖率 |
|---|---:|---:|---:|---:|
| 前 50 大目录（PLAN 成功标准的口径） | 47（34 + 13） | 3 | 0 | **100.0%**（目标 ≥ 80%） |
| 前 200 大目录（找待办用） | 172（80 + 92） | 19 | 9 | **95.5%** |

### 这个数字要怎么看（诚实说明）

1. **“容器”也算命中**：前 50 里有 15 个是 `scope = "self"` 的容器规则（`C:\Users`、`AppData\Local`、`Program Files`、`ProgramData`、`Packages` 等）。它们确实回答了“这是什么”，但回答是“里面混着各种东西，点进去看”。把这 15 个去掉，剩下 35 个具体目录也全部有规则或启发式说明。
2. **继承命中**：13 个是“上层文件夹命中了规则、子文件夹沿用”，比如回收站里的子目录、`Claude_…\LocalCache`。界面和 CLI 都会标“继承上层”，不会假装是精确命中。
3. **3 个启发式全是同一处**：`C:\Users\king\AppData\ACLOS\…`（6.3 GB 的 .mp4/.jpeg，名字叫 wonderfulVideos）。注册表里找不到对应软件，所以没有写规则；启发式 `media-files` 给出的是“一堆视频/图片/音频，可能是你自己的，也可能是软件自动录的，先打开看看”，档位 review。它是否算“有说明”见仁见智——如果不算，前 50 覆盖率是 94%。
4. **前 50 大目录是嵌套的**：`C:\Users` ⊃ `C:\Users\king` ⊃ `AppData` …，所以 50 行并不是 50 块不相交的空间。
5. **全盘按规则库归档**（safe / review / keep / 未覆盖）四项之和大于卷已用空间，因为阶段 A 已知的约 10 GB 重复计数（WinSxS 硬链接、商店版 Claude 目录映射）还没去重（阶段 C）。
6. 启发式和规则都**只看名字、大小、时间**，没有打开任何文件读内容。

## 附：前 50 大目录逐项解释（`diskwhy-cli explain C:\ --top 50` 原始输出）

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

### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 34 |
| 规则继承上层 | 13 |
| 启发式 | 3 |
| 未知 | 0 |
| **规则 + 启发式覆盖率** | **100.0%**（50/50） |

### 全盘按规则库归档（不含启发式）

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

### 未知项（下一批规则的待办）

（无）

## 附：前 200 大目录的统计与未知项（`--top 200`）

### 统计

| 来源 | 个数 |
|---|---:|
| 规则精确命中 | 80 |
| 规则继承上层 | 92 |
| 启发式 | 19 |
| 未知 | 9 |
| **规则 + 启发式覆盖率** | **95.5%**（191/200） |

### 未知项（下一批规则的待办）

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

### 未知项待办：下一批规则怎么写（我的判断，未核实的都标了）

| 路径 | 看到的线索 | 下一步 |
|---|---|---|
| `%APPDATA%\ACLOS`、`%USERPROFILE%\AppData\ACLOS` | Electron 程序（Cache / Code Cache / GPUCache 已被通配规则标绿），有 GameProxyCrashReports、TcVersions、Fixs，和 6 GB 的 wonderfulVideos 精彩时刻视频；注册表里没有登记 | 需要先确认是哪家的游戏加速/录制软件，再写规则：录像 → keep+migrate 或 review；TcVersions/Fixs → 待查 |
| `%USERPROFILE%\.vscode` | 只有 extensions（已有规则）+ argv.json/settings.json | 加容器规则（scope=self），很简单 |
| `%USERPROFILE%\.antigravity` | 同上，Antigravity 编辑器的扩展目录 | 同上 |
| `%USERPROFILE%\.cache\codex-runtimes` | dependencies 下有 native / python / node 三套运行时，runtime.json；另有 .previous-<GUID> 旧版本 | 看名字像 Codex 自带的运行时；需查 Codex 文档确认能否删、会不会自动重下 |
| `%LOCALAPPDATA%\OpenAI\Codex` | bin 里是 codex.exe、node.exe 和几个哈希命名的 exe 目录；runtimes | 看名字像 Codex 桌面/扩展的二进制；需确认它由谁安装、升级时旧版本是否残留 |

（另外，GUI 里 `AppData\Local` 的列表还能看到 kimi-desktop-updater（419 MB）、Pandoc（410 MB）等显示为“未知”的项，它们不在前 200 大目录里（第 200 名是 695 MB），没有计入上面的统计，也列为待办。）
