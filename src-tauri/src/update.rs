//! 自动更新:检查 GitHub latest release → 下载 portable zip → 原子换 exe → 重启。
//!
//! 刻意不引 HTTP 库:检查/下载用系统自带 curl.exe,解压用系统自带 tar.exe
//! (bsdtar 能直接解 zip),零新依赖、离线可编译。流程:
//! 1. GET api.github.com/repos/AbsoluteHLN/Taskasion/releases/latest,取 tag_name
//!    与 .zip 资产(优先 *-portable.zip),语义化比较版本号;
//! 2. 有新版 → 前端/托盘收到 update-status=available,用户点击后下载;
//! 3. 运行中的 exe 允许改名不允许覆盖 → 先改名让位为 .old,新 exe 落位,
//!    拉起新进程后退出;新进程启动时清理残留的 .old。

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

const REPO: &str = "AbsoluteHLN/Taskasion";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 检查到新版本后缓存的 (版本号, zip 下载地址),供 apply 使用。
static PENDING: Mutex<Option<(String, String)>> = Mutex::new(None);

fn curl_json() -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args([
            "-s",
            "--fail",
            "--max-time",
            "15",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: taskasion-updater",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("无法启动系统 curl.exe: {e}"))?;
    if !out.status.success() {
        return Err(format!("访问 GitHub 失败({})", out.status));
    }
    Ok(out.stdout)
}

/// 语义化版本三元组比较("1.0.2" > "1.0.1-aStart");预发布后缀忽略。
fn version_nums(v: &str) -> Vec<u32> {
    v.split(['.', '-', '+']).map_while(|p| p.parse::<u32>().ok()).collect()
}

fn is_newer(remote: &str, current: &str) -> bool {
    let (a, b) = (version_nums(remote), version_nums(current));
    for i in 0..3 {
        let ra = a.get(i).copied().unwrap_or(0);
        let rb = b.get(i).copied().unwrap_or(0);
        if ra != rb {
            return ra > rb;
        }
    }
    false
}

/// 查询 GitHub latest release → Some((新版本, zip 地址));已是最新 → None。
fn latest_release() -> Result<Option<(String, String)>, String> {
    let body = curl_json()?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("GitHub 响应解析失败: {e}"))?;
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("release 缺少 tag_name")?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets = v.get("assets").and_then(Value::as_array).ok_or("release 缺少 assets")?;
    let pick = |suffix: &str| -> Option<String> {
        assets.iter().find_map(|a| {
            let name = a.get("name").and_then(Value::as_str)?;
            let url = a.get("browser_download_url").and_then(Value::as_str)?;
            name.to_ascii_lowercase().ends_with(suffix).then(|| url.to_string())
        })
    };
    let url = pick("-portable.zip").or_else(|| pick(".zip")).ok_or("release 中没有 .zip 资产")?;
    Ok(Some((version, url)))
}

fn emit(app: &AppHandle, state: &str, version: Option<&str>, message: &str) {
    let _ = app.emit(
        "update-status",
        json!({ "state": state, "version": version, "message": message }),
    );
}

/// 检查更新并向前端发 update-status(latest / available / error)。
pub fn check_and_emit(app: &AppHandle) {
    match latest_release() {
        Ok(None) => emit(app, "latest", None, "已是最新版本"),
        Ok(Some((version, url))) => {
            *PENDING.lock().unwrap() = Some((version.clone(), url));
            emit(app, "available", Some(&version), &format!("发现新版本 {version}"));
        }
        Err(e) => emit(app, "error", None, &e),
    }
}

/// 启动 6 秒后静默检查一次(只在 available 时打扰前端)。
pub fn schedule_auto_check(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(6));
        check_and_emit(&app);
    });
}

fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args(["-L", "--fail", "--max-time", "600", "-o"])
        .arg(dest)
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("无法启动 curl.exe: {e}"))?;
    if !status.success() {
        return Err("下载失败,请检查网络".into());
    }
    Ok(())
}

/// 在解压目录里递归找新 exe(历史安装名 taskasion.exe / 现名 Taskasion.exe,大小写不敏感)。
fn find_exe(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe(&path) {
                return Some(found);
            }
        } else if entry.file_name().to_string_lossy().eq_ignore_ascii_case("Taskasion.exe") {
            return Some(path);
        }
    }
    None
}

/// 应用更新(在独立线程:下载 → 解压 → 换 exe → 拉起新进程 → 退出)。
pub fn apply(app: AppHandle) {
    std::thread::spawn(move || {
        let notify = |state: &str, msg: &str| emit(&app, state, None, msg);
        let Some((version, url)) = PENDING.lock().unwrap().clone() else {
            notify("error", "没有待安装的更新");
            return;
        };
        notify("downloading", "正在下载更新…");

        let tmp = std::env::temp_dir().join(format!("taskasion-update-{}", std::process::id()));
        let extract = tmp.join("extract");
        let _ = std::fs::remove_dir_all(&tmp);
        if let Err(e) = std::fs::create_dir_all(&extract) {
            notify("error", &format!("创建临时目录失败: {e}"));
            return;
        }
        let zip = tmp.join("taskasion-update.zip");
        if let Err(e) = download_to(&url, &zip) {
            notify("error", &e);
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        let ok = Command::new("tar")
            .args(["-xf"])
            .arg(&zip)
            .arg("-C")
            .arg(&extract)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&zip);
        if !ok {
            notify("error", "解压失败");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        let Some(new_exe) = find_exe(&extract) else {
            notify("error", "压缩包里没有找到 taskasion.exe");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        };

        let Some(new_exe) = find_exe(&extract) else {
            notify("error", "压缩包里没有找到可执行文件");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        };

        // 运行中的 exe 允许改名、不允许同名覆盖:先改名让位,再落位。
        // 新 exe 用压缩包里自己的文件名(Windows 文件名大小写不敏感,历史安装的
        // taskasion.exe 借一次自更新自然迁移为 Taskasion.exe,.old 由启动时清理)。
        let current = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                notify("error", &format!("无法定位当前程序: {e}"));
                return;
            }
        };
        let dir = match current.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                notify("error", "无法定位安装目录");
                return;
            }
        };
        let new_name = new_exe
            .file_name()
            .map(|n| n.to_owned())
            .unwrap_or_else(|| current.file_name().unwrap_or_default().to_owned());
        let target = dir.join(new_name);
        let backup = current.with_extension("exe.old");
        let _ = std::fs::remove_file(&backup);
        if let Err(e) = std::fs::rename(&current, &backup) {
            notify("error", &format!("备份当前程序失败: {e}"));
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        if let Err(e) = std::fs::copy(&new_exe, &target) {
            // 落位失败 → 回滚改名,保证旧版仍可启动
            let _ = std::fs::rename(&backup, &current);
            notify("error", &format!("安装新版本失败: {e}"));
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        let _ = std::fs::remove_dir_all(&tmp);
        match Command::new(&target).spawn() {
            Ok(_) => {
                emit(&app, "restarting", Some(&version), "更新完成,正在重启");
                app.exit(0);
            }
            Err(e) => {
                // 回滚:删掉落位失败的新文件,恢复旧 exe
                let _ = std::fs::remove_file(&target);
                let _ = std::fs::rename(&backup, &current);
                notify("error", &format!("启动新版本失败: {e}"));
            }
        }
    });
}

/// 启动时清理上次更新留下的 *.exe.old(此时已不被锁定;文件名大小写不敏感,新旧名通吃)。
pub fn cleanup_old() {
    if let Ok(cur) = std::env::current_exe() {
        let _ = std::fs::remove_file(cur.with_extension("exe.old"));
    }
}
