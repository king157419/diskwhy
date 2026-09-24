//! `builtin_tool` / `migrate` 的指引：给出可复制的命令并解释，或者说明去哪里改设置。
//! **DiskWhy 不运行这里的任何命令**，只展示；“打开系统存储设置”用 `ms-settings:storagesense`。

use crate::explain::{Explanation, How, Source, Tier};
use serde::{Deserialize, Serialize};

/// 允许由界面打开的系统设置页（白名单）。
pub const ALLOWED_SETTINGS: &[(&str, &str)] = &[
    ("ms-settings:storagesense", "系统 > 存储（存储感知 / 临时文件）"),
    ("ms-settings:appsfeatures", "应用 > 安装的应用"),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Guidance {
    pub title: String,
    /// 可复制的命令（不会被自动运行）。
    pub command: Option<String>,
    /// 这条命令要在哪里运行。
    pub shell: Option<String>,
    pub needs_admin: bool,
    pub explain: String,
    /// 可以一键打开的系统设置页（必须在 [`ALLOWED_SETTINGS`] 里）。
    pub settings_uri: Option<String>,
}

/// 打开白名单里的系统设置页（ShellExecuteW "open"）。不在白名单里的一律拒绝——这里不执行任何命令。
pub fn open_settings(uri: &str) -> Result<(), String> {
    if !ALLOWED_SETTINGS.iter().any(|(u, _)| *u == uri) {
        return Err(format!("不允许打开：{uri}"));
    }
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = crate::win::wide("open");
    let file = crate::win::wide(uri);
    // 文档：返回值 > 32 表示成功。
    let h = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    if h as isize > 32 {
        Ok(())
    } else {
        Err(format!("打不开 {uri}（ShellExecuteW 返回 {}）", h as isize))
    }
}

fn cmd(title: &str, command: &str, shell: &str, admin: bool, explain: &str) -> Guidance {
    Guidance {
        title: title.into(),
        command: Some(command.into()),
        shell: Some(shell.into()),
        needs_admin: admin,
        explain: explain.into(),
        settings_uri: None,
    }
}

fn howto(title: &str, explain: &str) -> Guidance {
    Guidance { title: title.into(), command: None, shell: None, needs_admin: false, explain: explain.into(), settings_uri: None }
}

fn storage_settings() -> Guidance {
    Guidance {
        title: "打开系统存储设置".into(),
        command: None,
        shell: None,
        needs_admin: false,
        explain: "Windows 自带的“存储 > 临时文件”可以清理更新缓存、传递优化文件、旧的 Windows 安装（Windows.old）、回收站等，由系统决定哪些能删。".into(),
        settings_uri: Some("ms-settings:storagesense".into()),
    }
}

/// 按解释给出指引。keep 和“不能直接删”的项都会有；safe/review 且可删的项也会附上软件自带的清理命令（如果有）。
pub fn guidance_for(e: &Explanation) -> Vec<Guidance> {
    let id = match &e.source {
        Source::Rule { id, .. } => id.as_str(),
        _ => "",
    };
    let mut g: Vec<Guidance> = Vec::new();
    match id {
        "winsxs" => {
            g.push(cmd(
                "先看看能省多少（只分析，不删）",
                "DISM /Online /Cleanup-Image /AnalyzeComponentStore",
                "以管理员身份运行的“命令提示符”或 PowerShell",
                true,
                "列出组件存储的实际大小和“可回收的程序包”数量；WinSxS 里大量是硬链接，资源管理器看到的大小比实际大。",
            ));
            g.push(cmd(
                "清理被新版本替换掉的组件",
                "DISM /Online /Cleanup-Image /StartComponentCleanup",
                "以管理员身份运行的“命令提示符”或 PowerShell",
                true,
                "这是微软提供的唯一安全清理方式。千万不要手动删 WinSxS 里的文件，会导致更新失败甚至无法开机。",
            ));
        }
        "hiberfil" => {
            g.push(cmd(
                "关闭休眠（会删掉 hiberfil.sys）",
                "powercfg /h off",
                "以管理员身份运行的“命令提示符”",
                true,
                "关闭后没有“休眠”，也没有“快速启动”（开机可能慢几秒）。想恢复就运行 powercfg /h on。笔记本经常合盖休眠的不建议关。",
            ));
        }
        "windows-old" | "windows-update-download" | "software-distribution" | "delivery-optimization" | "windows-error-reports"
        | "memory-dumps" | "system-volume-information" | "windows-temp" | "thumbnail-cache" | "directx-shader-cache" => {
            g.push(storage_settings());
            g.push(cmd(
                "或者用经典的“磁盘清理”",
                "cleanmgr /d C:",
                "开始菜单的“运行”（Win+R）或命令提示符",
                false,
                "打开后点“清理系统文件”能看到更多选项（Windows 更新清理、以前的 Windows 安装等）。勾哪些由你决定。",
            ));
        }
        "recycle-bin" => {
            g.push(howto("清空回收站", "在桌面右键“回收站” → “清空回收站”。清空前可以打开看看，确认没有需要还原的东西。清空之后无法恢复。"));
            g.push(storage_settings());
        }
        "conda-pkgs" => g.push(cmd(
            "让 conda 自己清理下载缓存",
            "conda clean --all",
            "Anaconda Prompt 或已经 conda init 过的终端",
            false,
            "删除下载的安装包、未使用的包和索引缓存；已经建好的环境不受影响。先加 --dry-run 可以只看不删。",
        )),
        "conda-envs" => g.push(howto(
            "把 conda 环境放到别的盘",
            "conda 环境不能直接挪位置（里面写死了路径）。做法：在 D 盘新建环境 conda create --prefix D:\\conda-envs\\名字 python=版本，装好需要的包后删掉旧环境（conda env remove -n 名字）；以后可以在 .condarc 里设置 envs_dirs 让新环境默认建在 D 盘。",
        )),
        "pnpm-store" => g.push(cmd("清理 pnpm 没被引用的包", "pnpm store prune", "终端", false, "只删不再被任何项目引用的包。")),
        "go-cache" => g.push(cmd("清理 Go 编译缓存", "go clean -cache", "终端", false, "下次编译会重新生成。")),
        "npm-cache" => g.push(cmd("让 npm 自己清理缓存", "npm cache clean --force", "终端", false, "下次安装会重新下载。")),
        "pip-cache" => g.push(cmd("让 pip 自己清理缓存", "pip cache purge", "终端", false, "下次安装会重新下载。")),
        "uv-cache" => g.push(cmd("让 uv 自己清理缓存", "uv cache clean", "终端", false, "下次安装会重新下载。")),
        "yarn-cache" => g.push(cmd("让 yarn 自己清理缓存", "yarn cache clean", "终端", false, "下次安装会重新下载。")),
        "nuget-cache" => g.push(cmd("让 NuGet 自己清理缓存", "dotnet nuget locals all --clear", "终端", false, "下次还原包会重新下载。")),
        "docker-data-vhdx" | "docker-wsl" => {
            g.push(howto(
                "把 Docker 的磁盘文件挪到别的盘",
                "打开 Docker Desktop → Settings（设置）→ Resources → Advanced → Disk image location，选一个 D 盘的文件夹，点 Apply & restart，Docker 会自己搬过去。别直接剪切 vhdx 文件。",
            ));
            g.push(cmd(
                "清理不用的镜像、容器和构建缓存",
                "docker system prune",
                "终端（Docker Desktop 运行时）",
                false,
                "会列出要删的东西并让你确认；加 -a 会连没被容器使用的镜像一起删。清理后 vhdx 文件不一定马上变小。",
            ));
        }
        "wsl-distro-vhdx" | "wsl-root" => {
            g.push(howto(
                "把 WSL 发行版挪到别的盘",
                "新版 WSL 可以直接：wsl --manage <发行版名> --move D:\\WSL\\<发行版名>（先用 wsl -l -v 看名字）。旧版：wsl --export <发行版名> D:\\backup\\x.tar → wsl --unregister <发行版名>（这一步会删掉原来的，务必确认导出成功）→ wsl --import <发行版名> D:\\WSL\\<发行版名> D:\\backup\\x.tar。",
            ));
            g.push(cmd("先看看有哪些发行版", "wsl -l -v", "终端", false, "只列出，不做任何修改。"));
        }
        "wechat-files" => g.push(howto(
            "改微信的文件保存位置",
            "打开微信 → 左下角“≡” → 设置 → 文件管理（或“存储空间”）→ 更改，选一个 D 盘的文件夹；微信会提示迁移已有的聊天文件。也可以在同一页面清理缓存。",
        )),
        "qq-files" => g.push(howto(
            "改 QQ 的文件保存位置",
            "打开 QQ → 设置 → 存储管理（新版 QQ）/ 文件管理（旧版），修改“聊天记录及文件保存位置”到 D 盘；也可以在“存储管理”里清理缓存。",
        )),
        "wecom-files" => g.push(howto("改企业微信的文件保存位置", "企业微信 → 设置 → 文件管理（或“存储”）→ 更改文件存储位置。")),
        _ => {}
    }
    // 通用兜底：没有专门指引但 how 里有工具 / 迁移时，把规则里的 how_detail 列出来。
    if g.is_empty() {
        if e.how.contains(&How::BuiltinTool) || e.how.contains(&How::Migrate) || e.how.contains(&How::Manual) {
            if !e.how_detail.is_empty() {
                g.push(howto(if e.how.contains(&How::Migrate) { "怎么迁到别的盘" } else { "怎么用自带的工具清理" }, &e.how_detail));
            }
            if e.how.contains(&How::BuiltinTool) && e.tier != Tier::Keep {
                g.push(storage_settings());
            }
        }
    }
    g
}
