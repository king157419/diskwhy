//! 已安装软件快照：读注册表 Uninstall 键（启动时读一次并缓存）。
//!
//! 读三处（learn.microsoft.com “Uninstall Registry Key” / “32-bit and 64-bit Application Data in the Registry”）：
//! - `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`（64 位视图，KEY_WOW64_64KEY）
//! - `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall`（32 位软件）
//! - `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`（按用户安装的软件）
//!
//! 每个子键取 DisplayName / Publisher / InstallLocation / DisplayIcon（REG_SZ 或 REG_EXPAND_SZ，
//! RegGetValueW 会自动展开环境变量）。没有 DisplayName 的子键跳过。

use serde::{Deserialize, Serialize};

/// 一个已安装软件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledApp {
    pub display_name: String,
    pub publisher: String,
    /// 原样的安装目录（可能为空）。
    pub install_location: String,
    /// 规范化后的安装目录（小写、去引号、去结尾反斜杠；为空或太浅时为空）。
    pub location_norm: String,
    pub name_norm: String,
    pub publisher_norm: String,
    /// 来源：HKLM64 / HKLM32 / HKCU
    pub hive: String,
}

/// 注册表快照。
#[derive(Debug, Clone, Default)]
pub struct RegistrySnapshot {
    pub apps: Vec<InstalledApp>,
    pub load_ms: u64,
}

/// 名字规范化：小写、去括号内容、去版本号、去公司后缀、只留字母数字和汉字。
pub fn normalize_name(s: &str) -> String {
    let lower = s.to_lowercase();
    // 去掉括号里的内容：“Python 3.12 (64-bit)” → “python 3.12 ”
    let mut no_paren = String::with_capacity(lower.len());
    let mut depth = 0i32;
    for ch in lower.chars() {
        match ch {
            '(' | '（' | '[' => depth += 1,
            ')' | '）' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => no_paren.push(ch),
            _ => {}
        }
    }
    const SUFFIXES: &[&str] = &[
        "inc", "inc.", "ltd", "ltd.", "llc", "corp", "corp.", "corporation", "co", "co.", "gmbh", "limited", "company",
        "technologies", "technology", "software", "sa", "s.a.", "ag", "bv", "b.v.", "x64", "x86", "64-bit", "32-bit",
    ];
    let words: Vec<&str> = no_paren
        .split(|c: char| c.is_whitespace() || c == ',' || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .filter(|w| !SUFFIXES.contains(w))
        // 版本号：纯数字和点，或 v 开头跟数字
        .filter(|w| {
            let t = w.trim_start_matches('v');
            !(t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
        .collect();
    let mut out: String = words.concat().chars().filter(|c| c.is_alphanumeric()).collect();
    for cn in ["股份有限公司", "有限责任公司", "有限公司", "科技", "网络"] {
        if out.len() > cn.len() {
            out = out.replace(cn, "");
        }
    }
    out
}

/// 路径规范化：小写、去引号、正斜杠转反斜杠、去结尾反斜杠。
pub fn normalize_path(s: &str) -> String {
    let mut p = s.trim().trim_matches('"').replace('/', "\\").to_lowercase();
    while p.len() > 3 && p.ends_with('\\') {
        p.pop();
    }
    p
}

/// 太浅（盘根、Program Files 本身、Windows、用户目录根、AppData 根）的安装目录不能用来归属。
fn too_shallow(loc: &str) -> bool {
    let segs: Vec<&str> = loc.split('\\').filter(|s| !s.is_empty()).collect();
    if segs.len() <= 1 {
        return true;
    }
    let last = segs[segs.len() - 1];
    matches!(
        last,
        "program files" | "program files (x86)" | "windows" | "system32" | "programdata" | "users" | "appdata" | "local" | "roaming" | "programs"
    ) || (segs.len() == 3 && segs[1] == "users")
        || segs.get(1).is_some_and(|s| *s == "windows")
}

impl InstalledApp {
    pub fn new(display_name: &str, publisher: &str, install_location: &str, display_icon: &str, hive: &str) -> Self {
        let mut loc = normalize_path(install_location);
        if loc.is_empty() && !display_icon.trim().is_empty() {
            // DisplayIcon 形如 "C:\Program Files\App\app.exe,0"：取 exe 所在目录。
            let icon = display_icon.trim().trim_matches('"');
            let icon = icon.split(',').next().unwrap_or("");
            let icon = normalize_path(icon);
            if icon.ends_with(".exe") {
                if let Some((dir, _)) = icon.rsplit_once('\\') {
                    loc = dir.to_string();
                }
            }
        }
        if too_shallow(&loc) {
            loc.clear();
        }
        InstalledApp {
            display_name: display_name.trim().to_string(),
            publisher: publisher.trim().to_string(),
            install_location: install_location.trim().to_string(),
            location_norm: loc,
            name_norm: normalize_name(display_name),
            publisher_norm: normalize_name(publisher),
            hive: hive.to_string(),
        }
    }
}

impl RegistrySnapshot {
    pub fn from_apps(apps: Vec<InstalledApp>) -> Self {
        RegistrySnapshot { apps, load_ms: 0 }
    }

    /// 读真实注册表（非 Windows 返回空）。
    pub fn load() -> Self {
        let start = std::time::Instant::now();
        #[cfg(windows)]
        let apps = win::read_all();
        #[cfg(not(windows))]
        let apps = Vec::new();
        RegistrySnapshot { apps, load_ms: start.elapsed().as_millis() as u64 }
    }
}

/// 当前用户商店应用包的安装目录（见 `win::package_root_folders`）。非 Windows 返回空。
pub fn package_root_folders() -> Vec<String> {
    #[cfg(windows)]
    {
        win::package_root_folders()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod win {
    use super::InstalledApp;
    use std::collections::HashSet;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_32KEY, KEY_WOW64_64KEY, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// RegGetValueW 读字符串值（先问长度，ERROR_MORE_DATA 时重试一次）。
    unsafe fn get_string(key: HKEY, value: &str) -> Option<String> {
        let wv = wide(value);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        let mut buf: Vec<u16> = vec![0; 260];
        for _ in 0..3 {
            let mut cb: u32 = (buf.len() * 2) as u32;
            let rc = RegGetValueW(key, null(), wv.as_ptr(), flags, null_mut(), buf.as_mut_ptr() as *mut _, &mut cb);
            if rc == ERROR_SUCCESS {
                let n = (cb as usize / 2).min(buf.len());
                let s = &buf[..n];
                let end = s.iter().position(|&c| c == 0).unwrap_or(n);
                return Some(String::from_utf16_lossy(&s[..end]));
            } else if rc == ERROR_MORE_DATA {
                buf = vec![0; (cb as usize / 2) + 2];
            } else {
                return None;
            }
        }
        None
    }

    unsafe fn read_hive(root: HKEY, path: &str, sam_extra: u32, hive: &str, out: &mut Vec<InstalledApp>) {
        let wp = wide(path);
        let mut h: HKEY = null_mut();
        if RegOpenKeyExW(root, wp.as_ptr(), 0, KEY_READ | sam_extra, &mut h) != ERROR_SUCCESS {
            return;
        }
        let mut idx = 0u32;
        loop {
            // 注册表键名最长 255 个字符（Registry Element Size Limits）。
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let rc = RegEnumKeyExW(h, idx, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut());
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            idx += 1;
            if rc != ERROR_SUCCESS {
                continue;
            }
            let mut sub: HKEY = null_mut();
            if RegOpenKeyExW(h, name.as_ptr(), 0, KEY_READ | sam_extra, &mut sub) != ERROR_SUCCESS {
                continue;
            }
            if let Some(dn) = get_string(sub, "DisplayName").filter(|s| !s.trim().is_empty()) {
                let publ = get_string(sub, "Publisher").unwrap_or_default();
                let loc = get_string(sub, "InstallLocation").unwrap_or_default();
                let icon = get_string(sub, "DisplayIcon").unwrap_or_default();
                out.push(InstalledApp::new(&dn, &publ, &loc, &icon, hive));
            }
            RegCloseKey(sub);
        }
        RegCloseKey(h);
    }

    /// 当前用户已安装的商店应用包的安装目录（`PackageRootFolder`）。
    /// 位置：HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\<包全名>。
    /// 普通用户列不出 `Program Files\WindowsApps`，但每个包自己的目录是可读的——用这份清单去量。
    pub fn package_root_folders() -> Vec<String> {
        const REPO: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
        let mut out = Vec::new();
        unsafe {
            let wp = wide(REPO);
            let mut h: HKEY = null_mut();
            if RegOpenKeyExW(HKEY_CURRENT_USER, wp.as_ptr(), 0, KEY_READ, &mut h) != ERROR_SUCCESS {
                return out;
            }
            let mut idx = 0u32;
            loop {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                let rc = RegEnumKeyExW(h, idx, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut());
                if rc == ERROR_NO_MORE_ITEMS {
                    break;
                }
                idx += 1;
                if rc != ERROR_SUCCESS {
                    continue;
                }
                let mut sub: HKEY = null_mut();
                if RegOpenKeyExW(h, name.as_ptr(), 0, KEY_READ, &mut sub) != ERROR_SUCCESS {
                    continue;
                }
                if let Some(p) = get_string(sub, "PackageRootFolder").filter(|s| !s.trim().is_empty()) {
                    out.push(p);
                }
                RegCloseKey(sub);
            }
            RegCloseKey(h);
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn read_all() -> Vec<InstalledApp> {
        let mut out = Vec::new();
        unsafe {
            read_hive(HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY, "HKLM64", &mut out);
            read_hive(HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_32KEY, "HKLM32", &mut out);
            read_hive(HKEY_CURRENT_USER, UNINSTALL, 0, "HKCU", &mut out);
        }
        // 同名同位置去重（有的软件在两个视图里都登记）。
        let mut seen = HashSet::new();
        out.retain(|a| seen.insert((a.display_name.to_lowercase(), a.location_norm.clone())));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_names() {
        assert_eq!(normalize_name("Docker Desktop"), "dockerdesktop");
        assert_eq!(normalize_name("Python 3.12.4 (64-bit)"), "python");
        assert_eq!(normalize_name("Google LLC"), "google");
        assert_eq!(normalize_name("Microsoft Corporation"), "microsoft");
        assert_eq!(normalize_name("Tencent Technology (Shenzhen) Company Limited"), "tencent");
        assert_eq!(normalize_name("Ollama version 0.12.3"), "ollamaversion");
        assert_eq!(normalize_name("北京字跳网络技术有限公司"), "北京字跳技术");
    }

    #[test]
    fn install_location_from_icon_and_shallow_filter() {
        let a = InstalledApp::new("App", "Pub", "", "\"C:\\Program Files\\App\\app.exe\",0", "HKLM64");
        assert_eq!(a.location_norm, r"c:\program files\app");
        let b = InstalledApp::new("X", "", r"C:\Program Files\", "", "HKLM64");
        assert_eq!(b.location_norm, "");
        let c = InstalledApp::new("Y", "", r"C:\Users\bob", "", "HKCU");
        assert_eq!(c.location_norm, "");
        let d = InstalledApp::new("Z", "", "", r"C:\Windows\Installer\{1}\icon.exe", "HKLM64");
        assert_eq!(d.location_norm, "");
    }

    #[cfg(windows)]
    #[test]
    fn real_registry_has_some_apps() {
        let snap = RegistrySnapshot::load();
        assert!(!snap.apps.is_empty(), "本机 Uninstall 键里应该至少有一个软件");
        assert!(snap.apps.iter().all(|a| !a.display_name.is_empty()));
    }
}
