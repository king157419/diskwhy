//! 环境变量：收集规则里会用到的 `%VAR%`，并负责展开。
//!
//! 只认白名单里的变量（见 [`ALLOWED_VARS`]），规则里出现别的变量在校验阶段就会报错。

use std::collections::HashMap;

/// 规则路径模式允许使用的环境变量（大小写不敏感）。
pub const ALLOWED_VARS: &[&str] = &[
    "SystemDrive",
    "SystemRoot",
    "WINDIR",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramData",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "TEMP",
    "PUBLIC",
];

/// 另外收集、但不允许出现在规则里的变量（只用于“受保护目录”判断）。
const EXTRA_VARS: &[&str] = &["OneDrive", "OneDriveConsumer", "OneDriveCommercial", "USERNAME"];

/// 展开后的环境变量表。键为大写变量名，值去掉了结尾反斜杠。
#[derive(Debug, Clone, Default)]
pub struct EnvMap {
    vars: HashMap<String, String>,
}

fn clean_value(v: &str) -> String {
    let mut s = v.trim().replace('/', "\\");
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

impl EnvMap {
    /// 从当前进程读取。
    pub fn from_process() -> Self {
        let mut m = EnvMap::default();
        for name in ALLOWED_VARS.iter().chain(EXTRA_VARS) {
            if let Ok(v) = std::env::var(name) {
                if !v.trim().is_empty() {
                    m.set(name, &v);
                }
            }
        }
        // 兜底：个别环境（服务、精简 shell）可能缺变量。
        if m.get("SystemRoot").is_none() {
            if let Some(w) = m.get("WINDIR").map(str::to_string) {
                m.set("SystemRoot", &w);
            }
        }
        if m.get("WINDIR").is_none() {
            if let Some(w) = m.get("SystemRoot").map(str::to_string) {
                m.set("WINDIR", &w);
            }
        }
        m
    }

    /// 测试用：从键值对构造。
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut m = EnvMap::default();
        for (k, v) in pairs {
            m.set(k, v);
        }
        m
    }

    pub fn set(&mut self, name: &str, value: &str) {
        self.vars.insert(name.to_ascii_uppercase(), clean_value(value));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(&name.to_ascii_uppercase()).map(|s| s.as_str())
    }

    /// 展开 `%VAR%`。遇到未定义的变量返回 `Err(变量名)`。
    pub fn expand(&self, pattern: &str) -> Result<String, String> {
        let mut out = String::with_capacity(pattern.len() + 32);
        let mut rest = pattern;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('%').ok_or_else(|| format!("未闭合的 % ：{pattern}"))?;
            let name = &after[..end];
            let val = self.get(name).ok_or_else(|| name.to_string())?;
            out.push_str(val);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// 模式开头的 `%VAR%` 名（没有则 None）。
pub fn leading_var(pattern: &str) -> Option<&str> {
    let p = pattern.strip_prefix('%')?;
    let end = p.find('%')?;
    Some(&p[..end])
}

/// 模式里出现的所有 `%VAR%` 名。
pub fn vars_in(pattern: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut rest = pattern;
    while let Some(s) = rest.find('%') {
        let after = &rest[s + 1..];
        match after.find('%') {
            Some(e) => {
                v.push(&after[..e]);
                rest = &after[e + 1..];
            }
            None => break,
        }
    }
    v
}

pub fn is_allowed_var(name: &str) -> bool {
    ALLOWED_VARS.iter().any(|a| a.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_basic_and_case_insensitive_names() {
        let e = EnvMap::from_pairs(&[("LOCALAPPDATA", r"C:\Users\bob\AppData\Local\"), ("ProgramFiles(x86)", r"C:\Program Files (x86)")]);
        assert_eq!(e.expand(r"%localappdata%\npm-cache").unwrap(), r"C:\Users\bob\AppData\Local\npm-cache");
        assert_eq!(e.expand(r"%PROGRAMFILES(X86)%\Steam").unwrap(), r"C:\Program Files (x86)\Steam");
        assert_eq!(e.expand(r"%NOPE%\x").unwrap_err(), "NOPE");
        assert_eq!(leading_var(r"%APPDATA%\x"), Some("APPDATA"));
        assert_eq!(vars_in(r"%A%\b\%C%"), vec!["A", "C"]);
    }
}
