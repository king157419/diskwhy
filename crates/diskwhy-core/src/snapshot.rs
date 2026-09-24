//! 扫描结果快照：把完整的 [`ScanResult`] 存成文件，之后再打开复核（解释照常现算）。
//!
//! 文件格式：gzip 压缩的 JSON（扩展名建议 `.dwsnap`），内容为
//! `{"format":"diskwhy-snapshot","version":1,"saved_at":…,"app_version":…,"result":{ScanResult}}`。
//! 为了省空间，树里除根以外的节点都不写 `path`（加载时由父路径 + 名字拼回来）。
//!
//! 加载时也接受：未压缩的同格式 JSON；以及 `diskwhy-cli scan --json` 的输出（那是一个扁平的
//! ScanResult，树只截到几层——能打开，但更深的目录看不到）。

use crate::fs::join;
use crate::model::{Attribution, InaccessibleDir, MappedDir, Node, NodeKind, ScanResult, SizeMode};
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Serialize, Serializer};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

pub const FORMAT: &str = "diskwhy-snapshot";
pub const VERSION: u32 = 1;

/// 快照文件头信息（加载后给界面显示）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotMeta {
    pub format: String,
    pub version: u32,
    /// 保存时间（Unix 秒）。
    pub saved_at: i64,
    pub app_version: String,
    /// 来源说明：snapshot（完整快照）/ cli-json（CLI --json 输出，树可能被截断）。
    #[serde(default)]
    pub kind: String,
}

#[derive(Deserialize)]
struct SnapshotIn {
    format: String,
    version: u32,
    saved_at: i64,
    #[serde(default)]
    app_version: String,
    result: ScanResult,
}

/// 序列化时不写子节点 path 的包装。
struct SnapNode<'a> {
    n: &'a Node,
    root: bool,
}

impl Serialize for SnapNode<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let n = self.n;
        let mut st = s.serialize_struct("Node", 21)?;
        st.serialize_field("name", &n.name)?;
        if self.root {
            st.serialize_field("path", &n.path)?;
        }
        st.serialize_field("kind", &n.kind)?;
        st.serialize_field("logical", &n.logical)?;
        st.serialize_field("allocated", &n.allocated)?;
        st.serialize_field("mtime", &n.mtime)?;
        st.serialize_field("attrs", &n.attrs)?;
        if n.reparse_tag != 0 {
            st.serialize_field("reparse_tag", &n.reparse_tag)?;
        }
        if n.inaccessible {
            st.serialize_field("inaccessible", &true)?;
        }
        if n.error_code != 0 {
            st.serialize_field("error_code", &n.error_code)?;
        }
        if n.reparse_skipped {
            st.serialize_field("reparse_skipped", &true)?;
        }
        if n.cloud_placeholder {
            st.serialize_field("cloud_placeholder", &true)?;
        }
        if n.system_reserved {
            st.serialize_field("system_reserved", &true)?;
        }
        if n.hardlink_dup {
            st.serialize_field("hardlink_dup", &true)?;
        }
        if n.mapped {
            st.serialize_field("mapped", &true)?;
        }
        if n.rest_flags != 0 {
            st.serialize_field("rest_flags", &n.rest_flags)?;
        }
        st.serialize_field("files", &n.files)?;
        st.serialize_field("dirs", &n.dirs)?;
        if !n.children.is_empty() {
            st.serialize_field("children", &SnapChildren(&n.children))?;
        }
        st.end()
    }
}

struct SnapChildren<'a>(&'a [Node]);

impl Serialize for SnapChildren<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for c in self.0 {
            seq.serialize_element(&SnapNode { n: c, root: false })?;
        }
        seq.end()
    }
}

/// 与 [`ScanResult`] 字段一一对应，只是树换成 [`SnapNode`]。
/// （`snapshot_roundtrip` 测试保证两边不会漏字段：存 → 读 后逐字段相等。）
#[derive(Serialize)]
struct ResultOut<'a> {
    root: &'a str,
    total_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
    cluster_size: u64,
    tree: SnapNode<'a>,
    buckets: &'a Attribution,
    duration_ms: u64,
    files: u64,
    dirs: u64,
    elevated: bool,
    inaccessible_dirs: &'a [InaccessibleDir],
    inaccessible_total: u64,
    size_mode: SizeMode,
    top_files_per_dir: usize,
    mapped_dirs: &'a [MappedDir],
}

#[derive(Serialize)]
struct SnapshotOut<'a> {
    format: &'static str,
    version: u32,
    saved_at: i64,
    app_version: &'a str,
    result: ResultOut<'a>,
}

/// 写快照（gzip JSON）。返回写入的字节数。
pub fn save(r: &ScanResult, path: &Path, app_version: &str) -> Result<u64, String> {
    let f = std::fs::File::create(path).map_err(|e| format!("创建文件失败：{e}"))?;
    // serde_json 会写很多很小的片段：压缩器前面也要加缓冲，否则 deflate 被小块写拖得很慢。
    let gz = BufWriter::with_capacity(
        1 << 20,
        flate2::write::GzEncoder::new(BufWriter::with_capacity(1 << 20, f), flate2::Compression::fast()),
    );
    let out = SnapshotOut {
        format: FORMAT,
        version: VERSION,
        saved_at: crate::explain::now_unix(),
        app_version,
        result: ResultOut {
            root: &r.root,
            total_bytes: r.total_bytes,
            used_bytes: r.used_bytes,
            free_bytes: r.free_bytes,
            cluster_size: r.cluster_size,
            tree: SnapNode { n: &r.tree, root: true },
            buckets: &r.buckets,
            duration_ms: r.duration_ms,
            files: r.files,
            dirs: r.dirs,
            elevated: r.elevated,
            inaccessible_dirs: &r.inaccessible_dirs,
            inaccessible_total: r.inaccessible_total,
            size_mode: r.size_mode,
            top_files_per_dir: r.top_files_per_dir,
            mapped_dirs: &r.mapped_dirs,
        },
    };
    let mut w = gz;
    serde_json::to_writer(&mut w, &out).map_err(|e| format!("写快照失败：{e}"))?;
    let enc = w.into_inner().map_err(|e| format!("写快照失败：{e}"))?;
    let mut inner = enc.finish().map_err(|e| format!("写快照失败：{e}"))?;
    inner.flush().map_err(|e| format!("写快照失败：{e}"))?;
    drop(inner);
    std::fs::metadata(path).map(|m| m.len()).map_err(|e| e.to_string())
}

/// 读快照（自动识别 gzip / 纯 JSON / CLI --json 输出）。
pub fn load(path: &Path) -> Result<(ScanResult, SnapshotMeta), String> {
    let f = std::fs::File::open(path).map_err(|e| format!("打开文件失败：{e}"))?;
    let mut rd = BufReader::with_capacity(1 << 20, f);
    let mut head = [0u8; 2];
    let n = rd.read(&mut head).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    buf.extend_from_slice(&head[..n]);
    rd.read_to_end(&mut buf).map_err(|e| format!("读取失败：{e}"))?;
    let bytes = if n == 2 && head == [0x1f, 0x8b] {
        let mut out = Vec::with_capacity(buf.len() * 6);
        flate2::read::GzDecoder::new(&buf[..]).read_to_end(&mut out).map_err(|e| format!("解压失败：{e}"))?;
        out
    } else {
        buf
    };
    from_json_bytes(&bytes)
}

/// 从 JSON 字节解析（供测试和 GUI 复用）。
pub fn from_json_bytes(bytes: &[u8]) -> Result<(ScanResult, SnapshotMeta), String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let (mut r, meta) = if head.contains(FORMAT) {
        let s: SnapshotIn = parse(bytes).map_err(|e| format!("快照格式不对：{e}"))?;
        if s.format != FORMAT {
            return Err(format!("不是 DiskWhy 快照（format = {}）", s.format));
        }
        if s.version > VERSION {
            return Err(format!("快照版本 {} 比本程序支持的 {} 新，请升级 DiskWhy", s.version, VERSION));
        }
        let meta = SnapshotMeta {
            format: s.format,
            version: s.version,
            saved_at: s.saved_at,
            app_version: s.app_version,
            kind: "snapshot".into(),
        };
        (s.result, meta)
    } else {
        let r: ScanResult = parse(bytes).map_err(|e| format!("既不是快照也不是 diskwhy-cli --json 输出：{e}"))?;
        let meta = SnapshotMeta { format: "cli-json".into(), version: 0, saved_at: 0, app_version: String::new(), kind: "cli-json".into() };
        (r, meta)
    };
    if r.tree.path.is_empty() {
        r.tree.path = r.root.clone();
    }
    fill_paths(&mut r.tree);
    Ok((r, meta))
}

/// 目录树可能很深：关掉 serde_json 默认 128 层的递归限制（调用方在大栈线程里跑，见 [`with_big_stack`]）。
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    de.disable_recursion_limit();
    let v = T::deserialize(&mut de)?;
    de.end()?;
    Ok(v)
}

/// 在 256 MB 栈（只是保留地址空间，按需提交）的线程里执行，防止很深的目录树递归爆栈。
pub fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("diskwhy-snapshot".into())
        .stack_size(256 << 20)
        .spawn(f)
        .expect("创建线程失败")
        .join()
        .expect("快照线程异常")
}

/// 把省掉的目录 path 拼回来。
pub fn fill_paths(n: &mut Node) {
    let parent = n.path.clone();
    for c in n.children.iter_mut() {
        if c.kind == NodeKind::Dir {
            if c.path.is_empty() {
                c.path = join(&parent, &c.name);
            }
            fill_paths(c);
        }
    }
}
