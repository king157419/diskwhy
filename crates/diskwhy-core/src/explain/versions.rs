//! 旧版本规则（`keep_latest`）：同一个父文件夹里一堆版本兄弟，哪些是“正在用的版本”。
//!
//! 规则见 [`super::rules`] 文件头。这里只有纯函数（名字 + 修改时间 → 哪些算正在用），方便单测。

use std::cmp::Ordering;

/// 版本号里数字核心之后的一段（按数字 / 字母分段）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Tok {
    Num(u64),
    Str(String),
}

#[derive(Debug, Clone)]
struct VerKey {
    /// 至少两段数字（1.2 / 1.2.3 / 12.1.0.29162）。
    core: Vec<u64>,
    /// 核心后面没有 `-xxx` / `~xxx`（预发布）。
    release: bool,
    suffix: Vec<Tok>,
}

fn cmp_core(a: &[u64], b: &[u64]) -> Ordering {
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

fn cmp_ver(a: &VerKey, b: &VerKey) -> Ordering {
    cmp_core(&a.core, &b.core).then(a.release.cmp(&b.release)).then_with(|| a.suffix.cmp(&b.suffix))
}

fn tokens(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut num = false;
    let flush = |cur: &mut String, num: bool, out: &mut Vec<Tok>| {
        if !cur.is_empty() {
            out.push(if num { Tok::Num(cur.parse().unwrap_or(u64::MAX)) } else { Tok::Str(std::mem::take(cur)) });
            cur.clear();
        }
    };
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            if !num {
                flush(&mut cur, num, &mut out);
                num = true;
            }
            cur.push(ch);
        } else if ch.is_alphanumeric() {
            if num {
                flush(&mut cur, num, &mut out);
                num = false;
            }
            cur.push(ch);
        } else {
            flush(&mut cur, num, &mut out);
        }
    }
    flush(&mut cur, num, &mut out);
    out
}

/// `1.2.3`、`v1.2.3`、`1.2.3-beta.1`、`12.1.0.29162` → 排序键；不像版本号（哈希、GUID、只有一段数字）→ None。
fn parse(rest: &str) -> Option<VerKey> {
    let s = rest.strip_prefix(['v', 'V']).filter(|r| r.starts_with(|c: char| c.is_ascii_digit())).unwrap_or(rest);
    let b = s.as_bytes();
    let mut i = 0;
    let mut core = Vec::new();
    loop {
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == st || i - st > 18 {
            return None;
        }
        core.push(s[st..i].parse().ok()?);
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            i += 1;
            continue;
        }
        break;
    }
    if core.len() < 2 {
        return None;
    }
    let tail = &s[i..];
    if let Some(c) = tail.chars().next() {
        if c.is_alphanumeric() {
            return None; // 1.2.3abc：数字后面直接跟字母，多半是哈希
        }
    }
    Some(VerKey { core, release: !(tail.starts_with('-') || tail.starts_with('~')), suffix: tokens(tail) })
}

/// 共同前缀，截到第一个数字之前（`2.1.308` / `2.1.309` 不去任何东西，`app-1.0` / `app-2.0` 去掉 `app-`，
/// `v1.2` / `v1.3` 去掉 `v`）。前缀里本身带数字的（`x64-1.2`）就认不出版本号，改按修改时间，宁可保守。
fn common_prefix_len(names: &[String]) -> usize {
    let first: Vec<char> = names[0].chars().collect();
    let mut n = first.len();
    for s in &names[1..] {
        n = n.min(first.iter().zip(s.chars()).take_while(|(a, b)| **a == *b).count());
    }
    if let Some(d) = first[..n].iter().position(|c| c.is_ascii_digit()) {
        n = d;
    }
    first[..n].iter().map(|c| c.len_utf8()).sum()
}

/// 共同后缀（比如 `.exe`、`-win32-x64`），开头的数字 / `.数字` 不算。
fn common_suffix_len(rests: &[&str]) -> usize {
    let first: Vec<char> = rests[0].chars().collect();
    let mut n = first.len();
    for s in &rests[1..] {
        n = n.min(first.iter().rev().zip(s.chars().rev()).take_while(|(a, b)| **a == *b).count());
    }
    let suf = &first[first.len() - n..];
    let mut k = 0;
    loop {
        while k < suf.len() && suf[k].is_ascii_digit() {
            k += 1;
        }
        if k + 1 < suf.len() && suf[k] == '.' && suf[k + 1].is_ascii_digit() {
            k += 1;
            continue;
        }
        break;
    }
    suf[k..].iter().map(|c| c.len_utf8()).sum()
}

/// 名字都像版本号时返回各自的排序键（顺序同输入）；有一个不像就 None（改按修改时间）。
fn version_keys(names: &[String]) -> Option<Vec<VerKey>> {
    if names.is_empty() {
        return Some(Vec::new());
    }
    let p = common_prefix_len(names);
    let rests: Vec<&str> = names.iter().map(|s| &s[p..]).collect();
    let q = common_suffix_len(&rests);
    rests.iter().map(|r| parse(&r[..r.len() - q])).collect()
}

/// `items` = 同一父文件夹里命中同一条 keep_latest 规则的兄弟（名字, 修改时间）；返回每个是不是“正在用的版本”：
/// 版本号最高的 `n` 个（名字不都是版本号时按修改时间最新的 `n` 个），再加上修改时间最新的那一个。
pub fn pick_current(items: &[(String, i64)], n: usize) -> Vec<bool> {
    let mut cur = vec![false; items.len()];
    if items.is_empty() {
        return cur;
    }
    let names: Vec<String> = items.iter().map(|(s, _)| s.to_lowercase()).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    match version_keys(&names) {
        Some(keys) => order.sort_by(|&a, &b| cmp_ver(&keys[b], &keys[a]).then(items[b].1.cmp(&items[a].1)).then_with(|| names[b].cmp(&names[a]))),
        None => order.sort_by(|&a, &b| items[b].1.cmp(&items[a].1).then_with(|| names[b].cmp(&names[a]))),
    }
    for &i in order.iter().take(n.max(1)) {
        cur[i] = true;
    }
    // 修改时间最新的那一个永远留着（并列时取上面排得最靠前的，也就是版本号最高的）。
    let max_m = items.iter().map(|x| x.1).max().unwrap();
    if let Some(&newest) = order.iter().find(|&&i| items[i].1 == max_m) {
        cur[newest] = true;
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(items: &[(&str, i64)], n: usize) -> Vec<&'static str> {
        let v: Vec<(String, i64)> = items.iter().map(|(s, m)| (s.to_string(), *m)).collect();
        let cur = pick_current(&v, n);
        items.iter().zip(cur).filter(|(_, c)| *c).map(|((s, _), _)| &*Box::leak(s.to_string().into_boxed_str())).collect()
    }

    #[test]
    fn numeric_segments_compare_as_numbers() {
        // 修改时间都一样，只看版本号
        assert_eq!(pick(&[("1.9", 5), ("1.10", 5), ("1.2", 5)], 1), vec!["1.10"]);
        assert_eq!(pick(&[("app-2.0.0", 5), ("app-10.0.0", 5), ("app-9.9.9", 5)], 1), vec!["app-10.0.0"]);
        assert_eq!(pick(&[("2.1.308", 5), ("2.1.309", 5)], 1), vec!["2.1.309"]);
        assert_eq!(pick(&[("12.1.0.28485", 5), ("12.1.0.29162", 5)], 1), vec!["12.1.0.29162"]);
        assert_eq!(pick(&[("v1.2.3", 5), ("v1.12.0", 5)], 1), vec!["v1.12.0"]);
        // keep_latest = 2
        assert_eq!(pick(&[("1.9", 5), ("1.10", 5), ("1.2", 5), ("1.11", 5)], 2), vec!["1.10", "1.11"]);
    }

    #[test]
    fn release_beats_prerelease_and_common_suffix_is_ignored() {
        let items = [
            ("codex-command-runner-0.166.0-alpha.5.1.exe", 5),
            ("codex-command-runner-0.166.0.exe", 5),
            ("codex-command-runner-0.163.0.exe", 5),
            ("codex-command-runner-0.125.0-alpha.3.exe", 5),
        ];
        assert_eq!(pick(&items, 1), vec!["codex-command-runner-0.166.0.exe"]);
        assert_eq!(pick(&[("1.0.0-beta.2", 5), ("1.0.0-beta.10", 5)], 1), vec!["1.0.0-beta.10"]);
        assert_eq!(pick(&[("redhat.java-1.52.0-win32-x64", 5), ("redhat.java-1.55.0-win32-x64", 5)], 1), vec!["redhat.java-1.55.0-win32-x64"]);
    }

    #[test]
    fn non_version_names_fall_back_to_mtime() {
        // 哈希名字（数字开头也不算版本号）
        let items = [("5b9024f90663758b", 10), ("22cfb1d302613eae", 40), ("34ab3e1324cc55b5", 20)];
        assert_eq!(pick(&items, 1), vec!["22cfb1d302613eae"]);
        let guids = [
            ("codex-primary-runtime.previous-31450cd2-c150-4854-b46f-06c64e827b34", 1),
            ("codex-primary-runtime.previous-0492752e-1608-4b51-8e88-080d5fdd9ba9", 9),
            ("codex-primary-runtime.previous-041c8419-e871-47a9-b868-6b2d79ba5f9e", 7),
        ];
        assert_eq!(pick(&guids, 1), vec!["codex-primary-runtime.previous-0492752e-1608-4b51-8e88-080d5fdd9ba9"]);
        // 混着一个不像版本号的名字：整组按修改时间
        assert_eq!(pick(&[("1.2.0", 1), ("1.10.0", 2), ("latest", 3)], 1), vec!["latest"]);
        assert_eq!(pick(&[("1.2.0", 3), ("1.10.0", 2), ("latest", 1)], 2), vec!["1.2.0", "1.10.0"]);
        // 只有一段数字不算版本号
        assert_eq!(pick(&[("5", 9), ("12", 1)], 1), vec!["5"]);
    }

    #[test]
    fn newest_mtime_is_always_kept() {
        // 版本号最高的是 2.0，但 1.5 刚改过（比如还在用的旧版本刚写过东西）：两个都留
        assert_eq!(pick(&[("1.0", 1), ("1.5", 99), ("2.0", 50)], 1), vec!["1.5", "2.0"]);
        // 只有一个兄弟：留着
        assert_eq!(pick(&[("1.0", 1)], 1), vec!["1.0"]);
        assert!(pick_current(&[], 1).is_empty());
    }

    #[test]
    fn parse_rejects_hashes() {
        assert!(parse("1.2").is_some());
        assert!(parse("1.2.3abc").is_none());
        assert!(parse("0492752e-1608").is_none());
        assert!(parse("deadbeef").is_none());
        assert!(parse("1").is_none());
    }
}
