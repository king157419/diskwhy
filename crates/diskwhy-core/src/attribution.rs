//! 已用空间归属计算。
//!
//! 量得到的三块：
//! - `scanned`：普通文件 allocated 合计；
//! - `system_reserved`（量到部分）：卷根下 `$Recycle.Bin` / `System Volume Information` /
//!   `pagefile.sys` / `hiberfil.sys` / `swapfile.sys` / `Recovery` / `$WinREAgent` 里能读到的；
//! - `cloud_placeholder`：云占位文件本地占用（不打开文件，按 0 计）。
//!
//! 差额 `gap = used - 以上三块`，是扫描看不到的部分（无权限目录的内容、NTFS 元数据如 $MFT、
//! 系统保留目录里读不到的内容等）。分配规则：
//!
//! 1. `gap < 0`：量到的比卷已用还多——几乎总是硬链接被重复计数（WinSxS 等，阶段 C 去重），
//!    记为 `overcount`，其余差额类为 0；
//! 2. 扫描根不是卷根：差额 = 扫描范围之外的内容，记为 `unattributed`；
//! 3. 存在“非系统保留”的无权限目录：差额记为 `inaccessible`（这时差额里混着系统保留目录
//!    读不到的部分和 NTFS 元数据，无管理员权限无法再拆分，界面上明确这样说明）；
//! 4. 只有系统保留目录无权限：差额按 GetDiskFreeSpaceEx 差额兜底记入 `system_reserved`，
//!    同时写入 `system_reserved_estimated`；
//! 5. 没有任何无权限目录：差额记为 `unattributed`（主要是 NTFS 元数据）。
//!
//! 不变式：`scanned + inaccessible + system_reserved + cloud_placeholder + unattributed - overcount == used`。

use crate::model::Attribution;

/// 扫描过程中量到的数据。
#[derive(Debug, Clone, Default)]
pub struct Measured {
    pub scanned: u64,
    pub system_reserved: u64,
    pub cloud_alloc: u64,
    pub cloud_files: u64,
    pub cloud_logical: u64,
    pub inaccessible_dirs_system: u64,
    pub inaccessible_dirs_other: u64,
}

pub fn compute_attribution(used: u64, m: &Measured, root_is_volume: bool) -> Attribution {
    let measured_sum = m.scanned as i128 + m.system_reserved as i128 + m.cloud_alloc as i128;
    let gap = used as i128 - measured_sum;
    let mut a = Attribution {
        scanned: m.scanned,
        system_reserved: m.system_reserved,
        cloud_placeholder: m.cloud_alloc,
        cloud_placeholder_files: m.cloud_files,
        cloud_placeholder_logical: m.cloud_logical,
        gap: gap.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
        scope_is_volume_root: root_is_volume,
        inaccessible_dirs_system: m.inaccessible_dirs_system,
        inaccessible_dirs_other: m.inaccessible_dirs_other,
        ..Default::default()
    };
    if gap < 0 {
        a.overcount = (-gap) as u64;
    } else {
        let g = gap as u64;
        if !root_is_volume {
            a.unattributed = g;
        } else if m.inaccessible_dirs_other > 0 {
            a.inaccessible = g;
        } else if m.inaccessible_dirs_system > 0 {
            a.system_reserved += g;
            a.system_reserved_estimated = g;
        } else {
            a.unattributed = g;
        }
    }
    a
}

impl Attribution {
    /// 各项加总（减去重复计数），应当等于卷已用空间。
    pub fn total(&self) -> i128 {
        self.scanned as i128
            + self.inaccessible as i128
            + self.system_reserved as i128
            + self.cloud_placeholder as i128
            + self.unattributed as i128
            - self.overcount as i128
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(scanned: u64, sys: u64, inacc_sys: u64, inacc_other: u64) -> Measured {
        Measured {
            scanned,
            system_reserved: sys,
            inaccessible_dirs_system: inacc_sys,
            inaccessible_dirs_other: inacc_other,
            ..Default::default()
        }
    }

    #[test]
    fn gap_goes_to_inaccessible_when_other_dirs_denied() {
        let a = compute_attribution(1000, &m(600, 100, 1, 2), true);
        assert_eq!(a.inaccessible, 300);
        assert_eq!(a.unattributed, 0);
        assert_eq!(a.total(), 1000);
    }

    #[test]
    fn gap_falls_back_to_system_reserved() {
        let a = compute_attribution(1000, &m(600, 100, 1, 0), true);
        assert_eq!(a.system_reserved, 400);
        assert_eq!(a.system_reserved_estimated, 300);
        assert_eq!(a.total(), 1000);
    }

    #[test]
    fn gap_unattributed_when_everything_readable() {
        let a = compute_attribution(1000, &m(900, 50, 0, 0), true);
        assert_eq!(a.unattributed, 50);
        assert_eq!(a.total(), 1000);
    }

    #[test]
    fn overcount_when_measured_exceeds_used() {
        let a = compute_attribution(1000, &m(1100, 0, 0, 0), true);
        assert_eq!(a.overcount, 100);
        assert_eq!(a.total(), 1000);
    }

    #[test]
    fn non_root_scope_is_unattributed() {
        let a = compute_attribution(1000, &m(10, 0, 0, 3), false);
        assert_eq!(a.unattributed, 990);
        assert_eq!(a.inaccessible, 0);
        assert_eq!(a.total(), 1000);
    }
}
