//! DiskWhy 核心库：扫描引擎 + 归属统计（阶段 A）+ 解释引擎（阶段 B，见 [`explain`]）。
//!
//! 主要入口：
//! - [`scan::scan`]：扫描真实盘符 / 目录，返回 [`model::ScanResult`]（含完整目录树 `tree`）；
//! - [`scan::scan_with`]：同上，但文件系统后端可替换（测试用）；
//! - [`view::listing`]：按路径取某目录的子项视图（给 GUI / CLI 用）；
//! - [`view::top_dirs`]：全树前 N 大目录；
//! - [`win::list_fixed_drives`] / [`win::is_elevated`] / [`win::relaunch_as_admin`]。
//!
//! 本库不依赖 Tauri。

#[cfg(windows)]
pub mod action;
pub mod attribution;
pub mod dedup;
pub mod explain;
pub mod fs;
pub mod model;
pub mod scan;
pub mod snapshot;
pub mod view;
#[cfg(windows)]
pub mod win;

pub use model::*;
