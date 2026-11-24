//! `diskwhy-cli scan / clean / explain` 端到端测试：真的跑编译出来的 exe（子进程）。
//!
//! **所有删除 / 移动都只针对本测试自己在 `target\tmp` 里建的假目录树。** 子进程的环境变量改成：
//! `LOCALAPPDATA` / `USERPROFILE` → 假树里的 `local` / `home`（内置规则按它们展开，所以 `pip-cache`、`npm-cache`、
//! `whisper-models`、`chrome-profile` 这些真实规则命中的是假树），`DISKWHY_DATA_DIR` → 假树里的 `data`
//! （快照缓存、actions.log、perf.json、last-run.json 都写在这里，不碰真的 `%LOCALAPPDATA%\DiskWhy`）。
//! 扫描用 `--path <假树>`，不扫整个盘。回收站路径不在这里真的执行（只预演）。
#![cfg(windows)]

use serde_json::Value;
use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_diskwhy-cli");

struct Fx {
    root: PathBuf,
}

impl Fx {
    fn new(tag: &str) -> Self {
        let uniq = format!("{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos());
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("dwcli-{tag}-{uniq}"));
        for d in ["home", "local", "data"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let fx = Fx { root };
        // safe：pip 缓存（3 个文件）、npm 缓存（2 个文件）
        fx.old("local\\pip\\cache\\http\\a\\1.bin", 40_000);
        fx.old("local\\pip\\cache\\http\\b\\2.bin", 50_000);
        fx.old("local\\pip\\cache\\wheels\\3.whl", 60_000);
        fx.old("local\\npm-cache\\_cacache\\content-v2\\x.tgz", 70_000);
        fx.old("local\\npm-cache\\_cacache\\index-v5\\y", 8_000);
        // review：whisper 模型
        fx.old("home\\.cache\\whisper\\base.pt", 90_000);
        // keep：Chrome 用户资料
        fx.old("local\\Google\\Chrome\\User Data\\Default\\Bookmarks", 5_000);
        // 只能用工具清：pnpm store
        fx.old("local\\pnpm\\store\\v3\\files\\00\\abc", 20_000);
        fx
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn s(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.display().to_string()
        } else {
            self.p(rel).display().to_string()
        }
    }
    /// 写文件并把修改时间调到 2 小时前（避开“30 分钟内修改过”）。
    fn old(&self, rel: &str, n: usize) {
        self.fresh(rel, n);
        let f = fs::OpenOptions::new().write(true).open(self.p(rel)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(7200)).unwrap();
    }
    fn fresh(&self, rel: &str, n: usize) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![b'x'; n]).unwrap();
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args)
            .env("LOCALAPPDATA", self.p("local"))
            .env("USERPROFILE", self.p("home"))
            .env("DISKWHY_DATA_DIR", self.p("data"))
            .env_remove("OneDrive")
            .env_remove("OneDriveConsumer")
            .env_remove("OneDriveCommercial");
        c
    }
    /// 跑一次，返回 (退出码, stdout 解析成的 JSON, stderr)。stdout 必须整个是一个 JSON 文档。
    fn json(&self, args: &[&str]) -> (i32, Value, String) {
        let out = self.cmd(args).output().expect("运行 diskwhy-cli 失败");
        let stdout = String::from_utf8(out.stdout).expect("stdout 不是 UTF-8");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let v: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("stdout 不是单个 JSON（{e}）：\n{stdout}\n--- stderr ---\n{stderr}"));
        (out.status.code().unwrap_or(-1), v, stderr)
    }
    fn exists(&self, rel: &str) -> bool {
        self.p(rel).exists()
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn cand<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    v["candidates"].as_array().unwrap().iter().find(|c| c["id"] == id)
}

fn item<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    v["items"].as_array().unwrap().iter().find(|c| c["id"] == id)
}

const PIP: [&str; 3] = ["local\\pip\\cache\\http\\a\\1.bin", "local\\pip\\cache\\http\\b\\2.bin", "local\\pip\\cache\\wheels\\3.whl"];
const OTHERS: [&str; 3] = ["home\\.cache\\whisper\\base.pt", "local\\Google\\Chrome\\User Data\\Default\\Bookmarks", "local\\pnpm\\store\\v3\\files\\00\\abc"];

#[test]
fn scan_json_lists_candidates_and_caches() {
    let fx = Fx::new("scan");
    let (code, v, _) = fx.json(&["scan", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["schema"], "diskwhy.scan/1");
    assert_eq!(v["ok"], true);
    assert!(v["drive"]["total_bytes"].as_u64().unwrap() > 0);
    assert!(v["drive"]["free_bytes"].as_u64().unwrap() > 0);

    let pip = cand(&v, "pip-cache").expect("pip-cache 候选");
    assert_eq!(pip["tier"], "safe");
    assert_eq!(pip["cleanable"], true);
    assert_eq!(pip["default_selected"], true);
    assert!(pip["cleanable_bytes"].as_u64().unwrap() >= 150_000);
    assert_eq!(pip["paths"][0]["path"], fx.s("local\\pip\\cache"));
    assert_eq!(pip["allowed_to"], serde_json::json!(["delete", "recycle", "stash"]));
    for k in ["what", "lose", "comes_back", "label"] {
        assert!(!pip[k].as_str().unwrap().is_empty(), "pip-cache.{k} 为空");
    }

    let w = cand(&v, "whisper-models").expect("whisper-models 候选");
    assert_eq!(w["tier"], "review");
    assert_eq!(w["default_selected"], false);
    assert_eq!(w["allowed_to"], serde_json::json!(["recycle", "stash"]));
    assert!(w["lose_kind"].is_string());

    let pn = cand(&v, "pnpm-store").expect("pnpm-store 候选（只能用工具清）");
    assert_eq!(pn["cleanable"], false);
    assert!(cand(&v, "chrome-profile").is_none(), "keep 档不应出现在候选里");

    assert!(v["totals"]["safe"]["cleanable_bytes"].as_u64().unwrap() >= pip["cleanable_bytes"].as_u64().unwrap());
    assert!(v["totals"]["review"]["groups"].as_u64().unwrap() >= 1);
    let snap = v["snapshot"].as_str().expect("快照路径");
    assert!(Path::new(snap).starts_with(fx.p("data\\cli")), "快照应在 DISKWHY_DATA_DIR 下：{snap}");
    assert!(Path::new(snap).is_file());

    // 人看的输出：不带 --json 也能跑，列出 id
    let out = fx.cmd(&["scan", "--path", &fx.s(""), "--quiet"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("pip-cache") && text.contains("whisper-models"), "{text}");
}

#[test]
fn clean_dry_run_touches_nothing() {
    let fx = Fx::new("dry");
    let (code, _, _) = fx.json(&["scan", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 0);
    let (code, v, _) = fx.json(&["clean", "--safe", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["mode"], "dry_run");
    assert_eq!(v["scan"]["from"], "snapshot", "clean 应该用 scan 留下的快照，不重扫");
    assert!(v["result"].is_null());
    assert_eq!(v["can_execute"], true);
    let pip = item(&v, "pip-cache").expect("pip-cache 项");
    assert_eq!(pip["dest"], "delete");
    assert_eq!(pip["will_delete_count"], 3);
    assert!(item(&v, "npm-cache").is_some());
    assert!(item(&v, "whisper-models").is_none(), "--safe 不能带上 review 档");
    assert!(v["totals"]["will_delete_bytes"].as_u64().unwrap() >= 150_000);
    assert_eq!(v["strategy"]["safe"], "delete");
    assert_eq!(v["strategy"]["review"], "recycle");
    for f in PIP.iter().chain(OTHERS.iter()) {
        assert!(fx.exists(f), "预演不该动任何文件：{f}");
    }
    assert!(!fx.exists("data\\actions.log"), "预演不该写动作日志");

    // 人看的输出写明是预演
    let out = fx.cmd(&["clean", "--safe", "--path", &fx.s(""), "--quiet"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("DRY RUN") && text.contains("--yes"), "{text}");
}

#[test]
fn clean_yes_safe_deletes_only_safe_and_skips_hot_and_locked() {
    let fx = Fx::new("yes");
    // 刚改过的文件（30 分钟内）和被占用的文件：都应该跳过
    fx.fresh("local\\npm-cache\\_cacache\\tmp\\hot.bin", 10_000);
    fx.old("local\\pip\\cache\\http\\c\\locked.bin", 30_000);
    let held = fs::OpenOptions::new().read(true).share_mode(0).open(fx.p("local\\pip\\cache\\http\\c\\locked.bin")).unwrap();

    // 没扫过：clean 自己扫
    let (code, v, _) = fx.json(&["clean", "--safe", "--yes", "--path", &fx.s(""), "--json", "--quiet"]);
    drop(held);
    assert_eq!(code, 3, "有跳过的应该是部分完成（3）：{v:#}");
    assert_eq!(v["mode"], "executed");
    assert_eq!(v["scan"]["from"], "fresh_scan");
    let r = &v["result"];
    assert_eq!(r["failed"], 0, "{r:#}");
    assert_eq!(r["ok"], 5, "3 个 pip + 2 个 npm 旧文件：{r:#}");
    assert_eq!(r["by_dest"]["permanent"]["count"], 5);
    assert_eq!(r["by_dest"]["recycle"]["count"], 0);
    let reasons = &r["skipped_by_reason"];
    assert_eq!(reasons["hot"]["count"], 1, "{r:#}");
    assert_eq!(reasons["locked"]["count"], 1, "{r:#}");
    for f in PIP {
        assert!(!fx.exists(f), "safe 文件应该删掉了：{f}");
    }
    assert!(fx.exists("local\\npm-cache\\_cacache\\tmp\\hot.bin"), "30 分钟内改过的不能删");
    assert!(fx.exists("local\\pip\\cache\\http\\c\\locked.bin"), "被占用的不能删");
    for f in OTHERS {
        assert!(fx.exists(f), "review / keep / 只能用工具的不能动：{f}");
    }
    // 跳过的原因在预检里也列出来了（占用它的是测试进程自己）
    let pip = item(&v, "pip-cache").unwrap();
    let sample = pip["skipped_sample"].as_array().unwrap();
    let locked = sample.iter().find(|s| s["code"] == "locked").expect("locked 样例");
    assert!(locked["processes"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains(&format!("PID {}", std::process::id()))), "{locked:#}");
    // 动作日志写在 DISKWHY_DATA_DIR
    let log = fs::read_to_string(fx.p("data\\actions.log")).expect("actions.log");
    assert!(log.lines().count() >= 5);
    assert!(Path::new(r["log_path"].as_str().unwrap()).starts_with(fx.p("data")));
    assert!(fx.exists("data\\last-run.json"));
}

#[test]
fn review_needs_ids_and_is_never_direct_deleted() {
    let fx = Fx::new("review");
    let base = fx.s("");
    // --to delete + review → 拒绝，什么都不做
    let (code, v, _) = fx.json(&["clean", "--ids", "whisper-models", "--to", "delete", "--yes", "--path", &base, "--json", "--quiet"]);
    assert_eq!(code, 2, "{v:#}");
    assert!(v["refused"][0]["reason"].as_str().unwrap().contains("review-tier"), "{v:#}");
    assert!(fx.exists(OTHERS[0]));

    // 默认去向：review → 回收站（只预演，不真的送回收站）
    let (code, v, _) = fx.json(&["clean", "--ids", "whisper-models", "--path", &base, "--json", "--quiet"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(item(&v, "whisper-models").unwrap()["dest"], "recycle");

    // 暂存到指定文件夹：真的执行，文件进暂存区、原位置没了
    let stash = fx.s("stash");
    fs::create_dir_all(&stash).unwrap();
    let (code, v, _) = fx.json(&["clean", "--ids", "whisper-models", "--to", "stash", "--stash-dir", &stash, "--yes", "--path", &base, "--json", "--quiet"]);
    assert!(code == 0 || code == 3, "{v:#}");
    assert_eq!(v["result"]["by_dest"]["quarantine"]["count"], 1, "{v:#}");
    assert!(!fx.exists(OTHERS[0]), "whisper 模型应该已经挪进暂存区");
    let manifest = v["result"]["manifest_path"].as_str().expect("暂存清单");
    assert!(Path::new(manifest).starts_with(fx.p("stash")), "{manifest}");
    let moved = walk(&fx.p("stash")).into_iter().any(|p| p.file_name().is_some_and(|n| n == "base.pt"));
    assert!(moved, "暂存区里找不到 base.pt");
    // safe 档没被点名，没动
    for f in PIP {
        assert!(fx.exists(f));
    }
}

#[test]
fn keep_unknown_and_tool_only_are_refused() {
    let fx = Fx::new("refuse");
    let (code, v, _) = fx.json(&["clean", "--ids", "chrome-profile,no-such-thing,pnpm-store", "--yes", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 2, "{v:#}");
    let refused = v["refused"].as_array().unwrap();
    let why = |id: &str| refused.iter().find(|r| r["id"] == id).unwrap()["reason"].as_str().unwrap().to_string();
    assert!(why("chrome-profile").contains("keep"), "{}", why("chrome-profile"));
    assert!(why("no-such-thing").contains("unknown id"));
    assert!(why("pnpm-store").contains("cannot be deleted directly"));
    for f in PIP.iter().chain(OTHERS.iter()) {
        assert!(fx.exists(f));
    }
    // 混着一个能清的：能清的照常预演，退出码 3（部分）
    let (code, v, _) = fx.json(&["clean", "--ids", "pip-cache,chrome-profile", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 3, "{v:#}");
    assert!(item(&v, "pip-cache").is_some());
}

#[test]
fn explain_one_path() {
    let fx = Fx::new("explain");
    let (code, v, _) = fx.json(&["explain", &fx.s("local\\pip\\cache\\http"), "--json", "--quiet"]);
    assert_eq!(code, 0, "{v:#}");
    assert_eq!(v["schema"], "diskwhy.explain/1");
    assert_eq!(v["tier"], "safe", "继承上层 pip\\cache 的规则：{v:#}");
    assert_eq!(v["candidate_id"], "pip-cache");
    assert_eq!(v["tree_from"], "scan");

    let (code, v, _) = fx.json(&["explain", &fx.s("local\\Google\\Chrome\\User Data"), "--json", "--quiet"]);
    assert_eq!(code, 0);
    assert_eq!(v["tier"], "keep");
    assert_eq!(v["cli_cleanable"], false);
    assert!(v["cli_refusal"].is_string());

    // 扫过之后用快照
    let (code, _, _) = fx.json(&["scan", "--path", &fx.s(""), "--json", "--quiet"]);
    assert_eq!(code, 0);
    let (_, v, _) = fx.json(&["explain", &fx.s("home\\.cache\\whisper"), "--json", "--quiet"]);
    assert_eq!(v["tree_from"], "snapshot");
    assert_eq!(v["tier"], "review");
}

#[test]
fn usage_errors_exit_1_with_json_error() {
    let fx = Fx::new("usage");
    for args in [vec!["clean", "--json"], vec!["frobnicate", "--json"], vec!["clean", "--safe", "--to", "shred", "--json"], vec!["scan", "--drive", "CC:", "--json"]] {
        let (code, v, _) = fx.json(&args);
        assert_eq!(code, 1, "{args:?} → {v}");
        assert_eq!(v["ok"], false);
        assert!(v["error"].is_string());
    }
    let out = fx.cmd(&["--help"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("EXIT CODES") && help.contains("DiskWhy 命令行"));
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.extend(walk(&p));
            } else {
                v.push(p);
            }
        }
    }
    v
}
