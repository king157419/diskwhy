//! diskwhy-cli：DiskWhy 的命令行前端（控制台程序，不依赖 Tauri / WebView）。
//!
//! 给 Agent（Claude Code / Codex 等）用的三个命令在 [`agent`]：
//!   diskwhy-cli scan    [--drive C: | --path DIR] [--json]
//!   diskwhy-cli clean   (--safe | --ids a,b) [--to delete|recycle|stash] [--yes] [--json]
//!   diskwhy-cli explain <path> [--json]
//! 开发 / 排查用的旧命令挪到了 `diskwhy-cli dev …`（[`legacy`]）。

mod agent;
mod explain_cmd;
mod legacy;
mod preflight_cmd;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("dev") {
        legacy::main(argv[1..].to_vec());
        return;
    }
    // 整棵 C 盘的树很深（递归遍历 / 释放），主线程 1 MB 的栈不够：放进 256 MB 栈的线程里跑（和快照读写同一个办法）。
    let code = diskwhy_core::snapshot::with_big_stack(move || agent::run(argv));
    std::process::exit(code);
}
