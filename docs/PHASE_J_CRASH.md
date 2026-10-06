# 阶段 J：2026-10-06 清理中途“闪退”分析（Fable，只读调查）

结论：**不是程序崩溃，是被外部结束**（约 85% 把握，凶手无法从磁盘证据确定）。
- `C:\Windows\appcompat\pca\PcaGeneralDb0.txt`：12:19:42.676Z `msedgewebview2.exe` 异常退出码 **0x8f**；12:19:52.787Z `D:\desk\claudex\13\DiskWhy-app\DiskWhy.exe` 退出码 **0x8f**；12:20:42Z 另一个 WebView2 0xffff7001。
- WER 正常工作但没有 diskwhy / webview2 的 AppCrash；Rust panic 退出码 101、abort 0xC000001D / 0xC0000409，都不是 143。actions.log 最后一行正好在 256 条一批的边界、格式完整 → 工作线程到最后都正常。
- 同一时刻 Claude 桌面版自动更新并重启（06:19:07 本地），06:19:53 有通知；只是相关，不是证据。
- 另一个可能（Fable 规划审查提出）：用户点了标题栏 ✕ —— 执行中 ✕ 没拦。PCA 退出码 0x8f 与“点 ✕ 正常退出(0)”不符，所以更像外部结束，但 ✕ 必须拦。

## 要改（R 线 harden 步骤照做）
1. 执行中拦关窗：`RunEvent::WindowEvent{CloseRequested}` 在 acting 时 `prevent_close` + 通知前端；`RunEvent::ExitRequested` 记日志。
2. WebView2 掉了要活下来并记录：`.setup()` 里 `with_webview` → `CoreWebView2().add_ProcessFailed(...)`（webview2-com 0.38，已在 Cargo.lock），浏览器进程退出时重建窗口；执行线程照常跑完。
3. 证据：actions.log `start`（pid、exe、父 pid、计划总数、策略）/ 每批前 `batch_begin` / 每 5–30 秒 `heartbeat`；下次启动发现没 summary → 提示，并可读 PcaGeneralDb0.txt 里的退出码。panic hook 写 crash.log（带 backtrace；release 改 `debug = "line-tables-only"`、`strip = "none"`、`split-debuginfo = "packed"`）。`SetUnhandledExceptionFilter` + `MiniDumpWriteDump`，返回 `EXCEPTION_CONTINUE_SEARCH`。前端 error / unhandledrejection → `log_frontend_error`。`acting` 不能永远卡 true（webview 重载后可重新附着：`action_status`）。
4. 可选：执行放到子进程（diskwhy-cli），GUI 被杀也不丢状态。
5. 日志分批文件 / 轮转（这次 138 MB）。

## 执行路径上的其他隐患
1. 回收站超上限：C 盘回收站 24.4 GB / 26 万项，上限 12,238 MB；Explorer 下次回收时会**先永久删掉最早的（用户自己以前删的东西）**。预检要比较 计划字节 + `SHQueryRecycleBinW` 当前用量 vs `MaxCapacity`（卷 GUID 用 `GetVolumeNameForVolumeMountPointW`），超了就拒绝 / 改去暂存区。
2. `FOF_WANTNUKEWARNING` 在没有属主窗口、没有消息循环的 STA 线程上：单个文件大于上限或 NukeOnDelete 时会弹一个看不见的模态框，工作线程永远卡住，点“是”就永久删。去掉这个标志、超上限的文件预先分流、`SetOwnerWindow(hwnd)`、用 `IFileOperationProgressSink`（`PostDeleteItem` 的 hrDelete）拿每条真实结果。
3. 444 条“回收站操作被中止”其实是预检和执行之间又被占用的 Chromium 缓存（index、data_0..3）和 pywin32 的 .pyd/.dll；现在的提示错怪用户“点了否”。用 progress sink 修。
4. 逐文件回收 30 万条：~85 个/秒、60 万个 $R/$I，回收站界面 / 清空 / 还原几乎不可用。整夹回收或默认走暂存区。
5. 删掉正在运行的 Chromium 程序的缓存，包括 DiskWhy 自己的 `com.diskwhy.desktop\EBWebView`：排除自己的 EBWebView；拥有进程在运行的 `Cache_Data` / `GPUCache` 用 Restart Manager 查 `index` / `data_*` 后排除。
6. `std::thread::scope` 工作线程默认 2 MiB 栈，跑 shell32 + 外壳扩展偏薄：`Builder::new().stack_size(8 << 20).spawn_scoped`。
