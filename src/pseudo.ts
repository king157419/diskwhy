// 方块图根层“归属色块”（无权限 / 系统保留推算 / 未归属 / 云端占位）的说明。
// 它们不是真实文件夹，所以在前端合成一个 NodeView 给详情面板显示。
import type { Explanation, NodeView, PseudoKey, ScanSummary } from "./api";
import { fmtInt, humanBytes } from "./format";

const LABEL: Record<PseudoKey, string> = {
  inacc: "无权限（推算）",
  sysest: "系统保留（推算）",
  unattr: "未归属",
  cloud: "云端占位",
};

export function pseudoView(key: PseudoKey, s: ScanSummary): NodeView {
  const b = s.buckets;
  const bytes = { inacc: b.inaccessible, sysest: b.system_reserved_estimated, unattr: b.unattributed, cloud: b.cloud_placeholder }[key];
  const dirs = b.inaccessible_dirs_other + b.inaccessible_dirs_system;
  const sample = s.inaccessible_sample.slice(0, 12).map((d) => `${d.path}（错误 ${d.error_code}${d.error_code === 5 ? "：拒绝访问" : ""}）`);
  const adminTip = s.elevated
    ? "这次已经是管理员身份了，剩下读不到的一般是系统专用或正在使用的目录。"
    : "以管理员身份重新启动 DiskWhy（顶部“以管理员重新启动”）可以看到更多，这一块会明显变小。";

  const base: Omit<Explanation, "what" | "why" | "lose" | "how_detail" | "clues" | "notes"> = {
    tier: "keep",
    owner: "Windows / 其他账户",
    category: "system",
    how: ["builtin_tool"],
    gain_bytes: 0,
    gain_note: `占 ${humanBytes(bytes)}，不能手动删`,
    source: { kind: "attribution" },
    confidence: 0.6,
    refs: [],
    verified_on: null,
  };

  let ex: Explanation;
  switch (key) {
    case "inacc":
      ex = {
        ...base,
        what: "当前权限读不到的文件夹里的内容（大小是推算的）",
        why:
          "这些文件夹只允许系统或管理员访问，例如 System Volume Information（系统还原点 / 卷影副本）、" +
          "Windows\\System32\\config（注册表）、ProgramData 里 Defender 和搜索索引的数据、其他用户的文件夹等。" +
          "DiskWhy 量不到它们，只能用“卷已用空间 − 量到的部分”来推算，所以这里也包含 NTFS 自己的元数据（$MFT、日志等）。",
        lose: "这些是系统或其他账户的数据，不要手动删。",
        how_detail:
          adminTip +
          " 系统还原点可以在“控制面板 > 系统 > 系统保护 > 配置”里调小上限或删除；搜索索引可以在“索引选项”里重建或移动。",
        notes: [
          `共 ${fmtInt(dirs)} 个目录读不了（其中 ${fmtInt(b.inaccessible_dirs_system)} 个在系统保留项下）。`,
          b.dedup_enabled
            ? `已先去掉硬链接和商店应用目录映射造成的重复（${humanBytes(b.hardlink_dedup + b.mapped_dedup)}），所以这个推算值是可信的上限。`
            : "这份结果没有做去重（旧快照），重复计数会把这一块压小，数字仅供参考。",
          "Program Files\\WindowsApps 本身列不出来，但当前用户已安装的商店应用包目录可以读，已经按注册表里的包清单量上了。",
        ],
        clues: sample,
      };
      break;
    case "sysest":
      ex = {
        ...base,
        what: "系统保留目录里读不到的部分（推算）",
        why: "只有系统保留目录（回收站里其他用户的部分、卷影副本等）读不了，差额就记在这里。",
        lose: "系统管理的数据，不要手动删。",
        how_detail: adminTip,
        notes: [],
        clues: sample,
      };
      break;
    case "unattr":
      ex = {
        ...base,
        owner: "NTFS 文件系统",
        what: "说不清归属的部分",
        why: s.buckets.scope_is_volume_root
          ? "所有文件夹都读到了，但卷已用空间比量到的多：主要是 NTFS 元数据（$MFT、$LogFile、$UsnJrnl 等），以及扫描期间新写进来的数据。"
          : "扫描的不是整个盘，这一块是扫描范围之外的内容。",
        lose: "文件系统自身的数据，不能删。",
        how_detail: "不需要处理。",
        notes: [],
        clues: [],
      };
      break;
    default:
      ex = {
        ...base,
        owner: "OneDrive 等云同步",
        what: "只在云端的文件（本地不占空间）",
        why: `${fmtInt(b.cloud_placeholder_files)} 个文件只在云端，逻辑大小 ${humanBytes(b.cloud_placeholder_logical)}。DiskWhy 只看属性，没有打开它们，不会触发下载。`,
        lose: "不占本地空间，无需处理。",
        how_detail: "不需要处理。",
        notes: [],
        clues: [],
      };
  }

  return {
    name: LABEL[key],
    path: "",
    kind: "Dir",
    logical: bytes,
    allocated: bytes,
    mtime: 0,
    attrs: 0,
    reparse_tag: 0,
    inaccessible: key === "inacc",
    error_code: 0,
    reparse_skipped: false,
    cloud_placeholder: key === "cloud",
    system_reserved: key === "sysest",
    files: 0,
    dirs: key === "inacc" ? dirs : 0,
    child_count: 0,
    percent_of_parent: s.used_bytes ? (bytes * 100) / s.used_bytes : 0,
    type_label: "归属统计（不是真实文件夹）",
    explanation: ex,
    pseudo: key,
  };
}
