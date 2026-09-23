//! 路径模式：`*`（段内任意字符）、`?`（段内单个字符）、`**`（零个或多个整段）。
//!
//! 匹配按“段”进行（以 `\` 或 `/` 分隔），大小写不敏感（内部统一小写）。
//! 模式在加载时已展开环境变量。

/// 模式的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Lit(String),
    Glob(String),
    /// `**`
    Any,
}

/// 编译后的路径模式。
#[derive(Debug, Clone)]
pub struct Pattern {
    /// 展开后的原始字符串（用于显示）。
    pub expanded: String,
    pub segs: Vec<Seg>,
    /// 开头连续的字面段个数（用于剪枝）。
    pub prefix_len: usize,
    /// 含 `**` 时为 None（可以任意深）。
    pub max_len: Option<usize>,
    /// 具体程度，越大越具体。多条规则同时命中时取最大者。
    pub specificity: u32,
}

/// 把路径切成小写段。
pub fn split_lower(path: &str) -> Vec<String> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect()
}

impl Pattern {
    pub fn compile(expanded: &str) -> Pattern {
        let segs: Vec<Seg> = split_lower(expanded)
            .into_iter()
            .map(|s| {
                if s == "**" {
                    Seg::Any
                } else if s.contains('*') || s.contains('?') {
                    Seg::Glob(s)
                } else {
                    Seg::Lit(s)
                }
            })
            .collect();
        let prefix_len = segs.iter().take_while(|s| matches!(s, Seg::Lit(_))).count();
        let max_len = if segs.iter().any(|s| *s == Seg::Any) { None } else { Some(segs.len()) };
        let mut lit = 0u32;
        let mut glob = 0u32;
        let mut chars = 0u32;
        for s in &segs {
            match s {
                Seg::Lit(t) => {
                    lit += 1;
                    chars += t.chars().count() as u32;
                }
                Seg::Glob(t) => {
                    glob += 1;
                    chars += t.chars().filter(|c| *c != '*' && *c != '?').count() as u32;
                }
                Seg::Any => {}
            }
        }
        Pattern {
            expanded: expanded.to_string(),
            segs,
            prefix_len,
            max_len,
            specificity: (lit * 10 + glob * 4) * 1000 + chars.min(999),
        }
    }

    /// 完整匹配（`path` 已切成小写段）。
    pub fn matches(&self, path: &[String]) -> bool {
        if let Some(m) = self.max_len {
            if m != path.len() {
                return false;
            }
        }
        // 快速拒绝：最后一段是字面量时先比最后一段。
        if let (Some(Seg::Lit(l)), Some(p)) = (self.segs.last(), path.last()) {
            if l != p {
                return false;
            }
        }
        match_segs(&self.segs, path)
    }

    /// 这个模式有没有可能匹配 `dir`（小写段）之下的某个路径。用于遍历时剪枝。
    pub fn could_match_below(&self, dir: &[String]) -> bool {
        if let Some(m) = self.max_len {
            if m <= dir.len() {
                return false;
            }
        }
        let n = self.prefix_len.min(dir.len());
        self.segs[..n].iter().zip(&dir[..n]).all(|(s, d)| matches!(s, Seg::Lit(l) if l == d))
            && self.prefix_matches_loosely(dir)
    }

    /// 前缀之后的部分：逐段检查 dir 在模式里是否仍“走得通”（遇到 ** 即放行）。
    fn prefix_matches_loosely(&self, dir: &[String]) -> bool {
        for (i, d) in dir.iter().enumerate() {
            match self.segs.get(i) {
                None => return false,
                Some(Seg::Any) => return true,
                Some(Seg::Lit(l)) => {
                    if l != d {
                        return false;
                    }
                }
                Some(Seg::Glob(g)) => {
                    if !glob_match(g, d) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

fn match_segs(p: &[Seg], s: &[String]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(Seg::Any) => (0..=s.len()).any(|i| match_segs(&p[1..], &s[i..])),
        Some(Seg::Lit(l)) => !s.is_empty() && *l == s[0] && match_segs(&p[1..], &s[1..]),
        Some(Seg::Glob(g)) => !s.is_empty() && glob_match(g, &s[0]) && match_segs(&p[1..], &s[1..]),
    }
}

/// 段内通配：`*` 任意多个字符，`?` 一个字符。两边都应已小写。
pub fn glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, path: &str) -> bool {
        Pattern::compile(pat).matches(&split_lower(path))
    }

    #[test]
    fn literal_case_insensitive() {
        assert!(m(r"C:\Users\Bob\AppData\Local\npm-cache", r"c:\users\bob\appdata\local\NPM-CACHE"));
        assert!(!m(r"C:\Users\Bob\AppData\Local\npm-cache", r"C:\Users\Bob\AppData\Local\npm-cache\_cacache"));
        assert!(!m(r"C:\a\b", r"C:\a"));
    }

    #[test]
    fn star_within_segment() {
        assert!(m(r"C:\x\thumbcache_*.db", r"C:\x\thumbcache_1024.db"));
        assert!(!m(r"C:\x\thumbcache_*.db", r"C:\x\iconcache_16.db"));
        assert!(m(r"C:\u\*\Cache", r"C:\u\Default\Cache"));
        assert!(!m(r"C:\u\*\Cache", r"C:\u\a\b\Cache"));
        assert!(m(r"C:\p\Claude_*", r"C:\p\Claude_pzs8sxrjxfjjc"));
        assert!(glob_match("a?c", "abc") && !glob_match("a?c", "ac"));
    }

    #[test]
    fn double_star() {
        let p = Pattern::compile(r"C:\u\AppData\Roaming\**\Code Cache");
        assert!(p.matches(&split_lower(r"C:\u\AppData\Roaming\Code Cache")));
        assert!(p.matches(&split_lower(r"C:\u\AppData\Roaming\Code\Code Cache")));
        assert!(p.matches(&split_lower(r"C:\u\AppData\Roaming\LarkShell\aha\users\x\profile\Code Cache")));
        assert!(!p.matches(&split_lower(r"C:\u\AppData\Local\Code\Code Cache")));
        assert!(!p.matches(&split_lower(r"C:\u\AppData\Roaming\Code\Code Cache\js")));
    }

    #[test]
    fn specificity_prefers_more_literal() {
        let generic = Pattern::compile(r"C:\u\AppData\Local\**\Cache");
        let chrome = Pattern::compile(r"C:\u\AppData\Local\Google\Chrome\User Data\*\Cache");
        assert!(chrome.specificity > generic.specificity);
        let any_pkg = Pattern::compile(r"C:\u\Packages\*");
        let claude = Pattern::compile(r"C:\u\Packages\Claude_*");
        assert!(claude.specificity > any_pkg.specificity);
    }

    #[test]
    fn pruning() {
        let p = Pattern::compile(r"C:\u\AppData\Local\npm-cache");
        assert!(p.could_match_below(&split_lower(r"C:\u")));
        assert!(p.could_match_below(&split_lower(r"C:\u\AppData\Local")));
        assert!(!p.could_match_below(&split_lower(r"C:\u\AppData\Local\npm-cache")));
        assert!(!p.could_match_below(&split_lower(r"C:\Windows")));
        let g = Pattern::compile(r"C:\u\AppData\**\GPUCache");
        assert!(g.could_match_below(&split_lower(r"C:\u\AppData\Roaming\x\y\z")));
        assert!(!g.could_match_below(&split_lower(r"C:\u\Documents")));
        let w = Pattern::compile(r"C:\u\Packages\*\LocalState\ext4.vhdx");
        assert!(w.could_match_below(&split_lower(r"C:\u\Packages\Canonical")));
        assert!(!w.could_match_below(&split_lower(r"C:\u\Packages\Canonical\RoamingState")));
    }
}
