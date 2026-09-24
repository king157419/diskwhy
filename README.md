# DiskWhy

DiskWhy 是一个 Windows 磁盘空间分析工具：不只告诉你 C 盘哪里大，还要把已用空间的每一块都讲清楚——扫到的、没权限读的、系统保留的、云端占位的、说不清的，全部单独列出来，不悄悄少算。后续阶段会给每个目录加上“这是什么、属于哪个软件、删了会失去什么、怎么安全腾出来”的大白话说明，以及带护栏、可撤销的清理动作（见 [PLAN.md](PLAN.md)）。

当前进度：**阶段 B**（解释引擎：222 条规则 + 启发式 + 可选的 AI 推测；可放心清理的缓存规则下面会先检查有没有混着你的数据；只解释，不删除）。验证记录见 [docs/PHASE_A.md](docs/PHASE_A.md)、[docs/PHASE_B.md](docs/PHASE_B.md)，本机覆盖率见 [docs/coverage.md](docs/coverage.md)。

## 目录结构

```
crates/diskwhy-core   扫描、归属统计、解释引擎（纯 Rust 库，不依赖 Tauri，带测试）
rules                 规则库 TOML（编译期嵌入；另可放 %LOCALAPPDATA%\DiskWhy\rules\*.toml）
crates/diskwhy-cli    命令行 diskwhy-cli
src-tauri             Tauri 2 桌面壳（只做 IPC 转发）
src                   Vue 3 + TypeScript 前端
docs                  阶段验证记录与截图
```

## 环境要求

- Windows 10/11 x64
- Rust（MSVC 工具链，已在 1.93 上验证）
- Node.js 22 + npm 10
- WebView2 运行时（Windows 11 自带）

## 构建与运行

```powershell
# 测试核心库
cargo test -p diskwhy-core

# 命令行：扫描 C 盘，打印前 30 大目录和归属统计，并导出 JSON
cargo build --release -p diskwhy-cli
.\target\release\diskwhy-cli.exe scan C:\ --top 30 --json out.json

# 桌面程序（开发模式）
npm install
npm run tauri dev

# 桌面程序（发布版，产物在 target\release\diskwhy.exe）
npm run tauri build -- --no-bundle
```

命令行解释：`diskwhy-cli explain C:\ --top 50 [--json out.json] [--report r.json] [--md coverage.md] [--redact]`，逐个输出前 N 大目录的档位、来源和“这是什么”，并统计规则 / 启发式 / 未知各多少，以及按字节的叶子覆盖率；`--redact` 把 JSON 里的用户名换成 `<用户>`。

命令行其他参数：`--json-depth N`（JSON 里目录树保留几层，默认 3）、`--mode exact|fast`（默认 exact，每个文件都查实际占用）、`--threads N`、`--keep-files N`（每个目录保留前 N 大文件，默认 50）、`--quiet`。

以普通用户运行时，部分系统目录读不到，会记为“无权限”。界面顶部可一键“以管理员重新启动”。

## 许可

MIT，见 [LICENSE](LICENSE)。
