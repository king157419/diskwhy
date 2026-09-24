//! “用自带工具清理”时，工具叫什么（PLAN 第 2 节 `How::BuiltinTool(name)`）。
//!
//! 规则里 `how = "builtin_tool"` 的具体做法写在 `how_detail` 这段大白话里；界面要单独显示一个
//! 工具名，这里从 `how_detail` 里认出来：按下表找，**在文字里出现得最早**的那个就是主工具
//! （`how_detail` 都是先写推荐的办法、再写备选）。表里没有的，说明是软件自己设置里的清理，
//! 用“某软件自己的设置”。都认不出返回 None，界面只显示 `how_detail`。

/// (在 how_detail 里找的字样, 显示的工具名)。字样按不分大小写比较。
const TOOLS: &[(&str, &str)] = &[
    ("设置 > 系统 > 存储", "Windows 设置里的“存储”"),
    ("磁盘清理", "磁盘清理（cleanmgr）"),
    ("cleanmgr", "磁盘清理（cleanmgr）"),
    ("DISM", "DISM 命令（管理员终端）"),
    ("powercfg", "powercfg 命令（管理员终端）"),
    ("pnputil", "pnputil 命令（管理员终端）"),
    ("回收站", "回收站"),
    ("系统保护", "系统保护（控制面板里的还原点设置）"),
    ("设置 > 应用", "Windows 设置里的“已安装的应用”"),
    ("索引选项", "索引选项（控制面板）"),
    ("Windows 安全中心", "Windows 安全中心"),
    ("Set-MpPreference", "PowerShell 命令 Set-MpPreference"),
    ("同步中心", "同步中心（控制面板）"),
    ("诊断和反馈", "Windows 设置里的“诊断和反馈”"),
    ("Delete-DeliveryOptimizationCache", "PowerShell 命令 Delete-DeliveryOptimizationCache"),
    ("npm cache", "npm cache clean"),
    ("pnpm store prune", "pnpm store prune"),
    ("yarn cache clean", "yarn cache clean"),
    ("bun pm cache", "bun pm cache rm"),
    ("pip cache", "pip cache purge"),
    ("uv cache", "uv cache clean"),
    ("conda clean", "conda clean"),
    ("go clean", "go clean"),
    ("dotnet nuget locals", "dotnet nuget locals"),
    ("Docker Desktop", "Docker Desktop 的设置"),
    ("Invalidate Caches", "IDE 里的 Invalidate Caches"),
    ("Visual Studio Installer", "Visual Studio Installer"),
    ("释放空间", "资源管理器右键“释放空间”"),
    ("Office 上传中心", "Office 上传中心"),
    ("NVIDIA 控制面板", "NVIDIA 控制面板"),
    ("媒体缓存", "Adobe 软件的“媒体缓存”首选项"),
];

/// 浏览器“清除浏览数据”一类的字样：工具名要带上是哪个浏览器。
const BROWSER_CLEAR: &[&str] = &["删除浏览数据", "清除浏览数据", "清除数据"];

/// 从 `how_detail` 里认出工具名。`owner` 用于“某软件自己的设置”。
pub fn tool_name(how_detail: &str, owner: Option<&str>) -> Option<String> {
    let hay = how_detail.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    let mut consider = |pos: usize, name: String| {
        if best.as_ref().is_none_or(|(p, _)| pos < *p) {
            best = Some((pos, name));
        }
    };
    for (needle, name) in TOOLS {
        if let Some(pos) = hay.find(&needle.to_lowercase()) {
            consider(pos, (*name).to_string());
        }
    }
    let owner = owner.map(short_owner).filter(|o| !o.is_empty());
    for needle in BROWSER_CLEAR {
        if let Some(pos) = hay.find(needle) {
            let name = match &owner {
                Some(o) => format!("{o} 的“清除浏览数据”"),
                None => "浏览器的“清除浏览数据”".to_string(),
            };
            consider(pos, name);
        }
    }
    // 软件自己菜单里的设置（“百度网盘 → 设置 → 传输”）：菜单用 →，Windows 设置路径用 >，
    // 所以只认“→ 设置”，也参与“谁先出现”的比较。
    if let Some(pos) = hay.find("→ 设置") {
        consider(pos, app_settings(owner.as_deref()));
    }
    if let Some((_, name)) = best {
        return Some(name);
    }
    // 其余提到“设置”的（“微信“设置 → 存储空间”里清理”、“客户端设置里清理缓存”）
    if hay.contains("设置") {
        return Some(app_settings(owner.as_deref()));
    }
    None
}

fn app_settings(owner: Option<&str>) -> String {
    match owner {
        Some(o) => format!("{o} 自己的设置"),
        None => "软件自己的设置".to_string(),
    }
}

/// “LarkShell（Chromium / Electron 程序）”这类带括号的归属只取括号前面。
fn short_owner(o: &str) -> String {
    let cut = o.find(['（', '(']).unwrap_or(o.len());
    o[..cut].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_tool_wins_and_owner_fallback() {
        assert_eq!(
            tool_name("以管理员身份打开终端，运行：DISM /Online /Cleanup-Image /StartComponentCleanup", None).as_deref(),
            Some("DISM 命令（管理员终端）")
        );
        // 先写的是“存储”，后面的磁盘清理是备选
        assert_eq!(
            tool_name("用“设置 > 系统 > 存储 > 临时文件”里的“Windows 更新清理”，或“磁盘清理 > 清理系统文件”。", None).as_deref(),
            Some("Windows 设置里的“存储”")
        );
        assert_eq!(
            tool_name("“磁盘清理 > 清理系统文件”里勾选；或 pnputil /enum-drivers", None).as_deref(),
            Some("磁盘清理（cleanmgr）")
        );
        assert_eq!(
            tool_name("关掉 Chrome 后删除；或在 Chrome 的“删除浏览数据”里勾选“缓存的图片和文件”。", Some("Google Chrome")).as_deref(),
            Some("Google Chrome 的“清除浏览数据”")
        );
        assert_eq!(
            tool_name("微信 → 设置 → 存储空间（文件管理）→ 清理缓存。", Some("微信（腾讯）")).as_deref(),
            Some("微信 自己的设置")
        );
        assert_eq!(tool_name("pnpm store prune", Some("pnpm")).as_deref(), Some("pnpm store prune"));
        assert_eq!(tool_name("自己看着办", None), None);
    }
}
