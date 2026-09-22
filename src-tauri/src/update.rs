//! 自动更新:检查 GitHub latest release → 下载 portable zip → 原子换 exe → 重启。
//!
//! 刻意不引 HTTP 库:检查/下载用系统自带 curl.exe,解压用系统自带 tar.exe
//! (bsdtar 能直接解 zip),零新依赖、离线可编译。系统 curl.exe 不读 Windows
//! 系统代理,直连失败时回退到注册表里用户设置的 WinINET 代理再试一次。流程:
//! 1. GET api.github.com/repos/AbsoluteHLN/Taskasion/releases/latest,取 tag_name
//!    与 .zip 资产(优先 *-portable.zip),语义化比较版本号(预发布后缀参与比较,
//!    beta.2 → beta.3、beta.2 → 1.1.0 正式版都能识别);
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

/// 读 WinINET 系统代理(系统 curl.exe 不会自己读,直连失败时用它兜底)。
/// ProxyServer 形如 "127.0.0.1:7897" 或按协议 "http=…;https=host:port"。
fn wininet_proxy() -> Option<String> {
    let q = |name: &str| -> Option<String> {
        let out = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
                "/v",
                name,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.contains(name))?
            .split_whitespace()
            .next_back()
            .map(String::from)
    };
    if q("ProxyEnable")?.trim() != "0x1" {
        return None;
    }
    let server = q("ProxyServer")?;
    let pick = server
        .split(';')
        .find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k.eq_ignore_ascii_case("https") || k.eq_ignore_ascii_case("http")).then(|| v.to_string())
        })
        .unwrap_or(server);
    (!pick.is_empty()).then(|| format!("http://{pick}"))
}

fn curl_json() -> Result<Vec<u8>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let attempt = |proxy: Option<&str>| -> Result<Vec<u8>, String> {
        let mut args: Vec<String> = vec!["-s".into(), "--fail".into(), "--max-time".into(), "15".into()];
        if let Some(p) = proxy {
            args.extend(["--proxy".into(), p.to_string()]);
        }
        args.extend([
            "-H".into(),
            "Accept: application/vnd.github+json".into(),
            "-H".into(),
            "User-Agent: taskasion-updater".into(),
            url.clone(),
        ]);
        let out = Command::new("curl")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("无法启动系统 curl.exe: {e}"))?;
        if !out.status.success() {
            let detail = String::from_utf8_lossy(&out.stderr);
            let detail = detail.trim();
            return Err(if detail.is_empty() {
                format!("访问 GitHub 失败({})", out.status)
            } else {
                format!("访问 GitHub 失败: {detail}")
            });
        }
        Ok(out.stdout)
    };
    // 先直连;失败再借系统代理试一次(都失败时报直连的错误,更接近根因)。
    match attempt(None) {
        Ok(body) => Ok(body),
        Err(direct_err) => match wininet_proxy() {
            Some(proxy) => attempt(Some(&proxy)).map_err(|_| direct_err),
            None => Err(direct_err),
        },
    }
}

/// 预发布标识:数字段或字母数字段;semver 规则:数字段 < 字母数字段。
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreId {
    Num(u32),
    Str(String),
}

/// 解析 "1.2.3-beta.2":核心三元组 + 可选预发布段(忽略 '+' 后的构建元数据)。
/// 容忍缺段/非数字段(按 0 处理),tag 形如 v1.2 已在上游去掉 v 前缀。
fn parse_version(v: &str) -> ([u32; 3], Option<Vec<PreId>>) {
    let v = v.split('+').next().unwrap_or(v);
    let (core_s, pre_s) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let mut core = [0u32; 3];
    for (i, p) in core_s.split('.').enumerate().take(3) {
        core[i] = p.parse().unwrap_or(0);
    }
    let pre = pre_s.map(|p| {
        p.split('.')
            .map(|id| id.parse::<u32>().map(PreId::Num).unwrap_or_else(|_| PreId::Str(id.to_string())))
            .collect::<Vec<_>>()
    });
    (core, pre)
}

/// 语义化比较:先比核心三元组;相同则正式版 > 预发布,同为预发布逐段比较
/// (beta.2 < beta.3、beta.2 < beta.10、beta < beta.1、rc > beta)。
fn is_newer(remote: &str, current: &str) -> bool {
    let (rc, rp) = parse_version(remote);
    let (cc, cp) = parse_version(current);
    if rc != cc {
        return rc > cc;
    }
    match (rp, cp) {
        (Some(a), Some(b)) => a > b,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// 从 /releases/latest 响应里取版本与 zip 地址;不比当前新 → (版本, None)。
fn pick_release(v: &Value) -> Result<(String, Option<(String, String)>), String> {
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("release 缺少 tag_name")?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok((version, None));
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
    Ok((version.clone(), Some((version, url))))
}

/// 查询 GitHub latest release → (线上最新版本, Some((新版本, zip 地址)) 有新版)。
fn latest_release() -> Result<(String, Option<(String, String)>), String> {
    let body = curl_json()?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("GitHub 响应解析失败: {e}"))?;
    pick_release(&v)
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
        Ok((latest, None)) => emit(app, "latest", None, &format!("已是最新版本(线上 v{latest})")),
        Ok((_, Some((version, url)))) => {
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

fn download_once(url: &str, dest: &Path, proxy: Option<&str>) -> Result<(), String> {
    let mut args: Vec<String> = vec!["-L".into(), "--fail".into(), "--max-time".into(), "600".into(), "-o".into()];
    if let Some(p) = proxy {
        args.extend(["--proxy".into(), p.to_string()]);
    }
    args.push(dest.to_string_lossy().into_owned());
    args.push(url.to_string());
    let status = Command::new("curl")
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("无法启动 curl.exe: {e}"))?;
    if !status.success() {
        return Err("下载失败,请检查网络".into());
    }
    Ok(())
}

/// releases/download 的对象存储直连常被重置,失败时同样回退系统代理。
fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    match download_once(url, dest, None) {
        Ok(()) => Ok(()),
        Err(direct_err) => match wininet_proxy() {
            Some(proxy) => download_once(url, dest, Some(&proxy)).map_err(|_| direct_err),
            None => Err(direct_err),
        },
    }
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

#[cfg(test)]
mod tests {
    use super::is_newer;
    use serde_json::Value;

    // 按 GitHub /releases/latest 的真实字段形状裁剪(实测响应),只留比较/选资产用到的键。
    const FAKE_NEWER: &str = r#"{
        "tag_name": "v99.0.0-beta.3",
        "draft": false,
        "assets": [
            {"name": "Taskasion-99.0.0-beta.3-portable.zip",
             "browser_download_url": "https://github.com/AbsoluteHLN/Taskasion/releases/download/v99.0.0-beta.3/Taskasion-99.0.0-beta.3-portable.zip"},
            {"name": "Source.zip", "browser_download_url": "https://example.com/src.zip"}
        ]
    }"#;
    const FAKE_SAME: &str = r#"{
        "tag_name": "v0.0.0",
        "draft": false,
        "assets": [
            {"name": "Taskasion-0.0.0-portable.zip",
             "browser_download_url": "https://github.com/AbsoluteHLN/Taskasion/releases/download/v0.0.0/Taskasion-0.0.0-portable.zip"}
        ]
    }"#;
    const FAKE_NO_PORTABLE: &str = r#"{
        "tag_name": "v99.0.0",
        "assets": [
            {"name": "Source code (zip)", "browser_download_url": "https://example.com/auto.zip"},
            {"name": "Taskasion-99.0.0.zip",
             "browser_download_url": "https://github.com/AbsoluteHLN/Taskasion/releases/download/v99.0.0/Taskasion-99.0.0.zip"}
        ]
    }"#;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn picks_portable_zip_of_newer_release() {
        let (latest, upd) = super::pick_release(&parse(FAKE_NEWER)).unwrap();
        assert_eq!(latest, "99.0.0-beta.3");
        let (version, url) = upd.expect("应识别为有新版本");
        assert_eq!(version, "99.0.0-beta.3");
        assert!(url.ends_with("-portable.zip"));
    }

    #[test]
    fn same_version_reports_no_update() {
        let (latest, upd) = super::pick_release(&parse(FAKE_SAME)).unwrap();
        assert_eq!(latest, "0.0.0");
        assert!(upd.is_none());
    }

    #[test]
    fn falls_back_to_plain_zip_asset() {
        let (_, upd) = super::pick_release(&parse(FAKE_NO_PORTABLE)).unwrap();
        let (_, url) = upd.expect("无 portable 时应退回任意 .zip");
        assert!(url.ends_with("/Taskasion-99.0.0.zip")); // 不能挑中 Source code (zip)
    }

    #[test]
    fn detects_prerelease_bumps() {
        // 旧实现把 beta.N 整段丢掉,同核心三元组一律视为"已是最新"——beta.2→beta.3 永远检不出
        assert!(is_newer("1.1.0-beta.3", "1.1.0-beta.2"));
        assert!(is_newer("1.1.0-beta.10", "1.1.0-beta.2")); // 数字比较,不是字典序
        assert!(!is_newer("1.1.0-beta.2", "1.1.0-beta.10"));
        assert!(!is_newer("1.1.0-beta.2", "1.1.0-beta.2"));
        assert!(!is_newer("1.1.0-beta.1", "1.1.0-beta.2"));
    }

    #[test]
    fn stable_beats_prerelease() {
        assert!(is_newer("1.1.0", "1.1.0-beta.2"));
        assert!(is_newer("1.1.0", "1.1.0-rc.1"));
        assert!(!is_newer("1.1.0-beta.2", "1.1.0"));
        assert!(!is_newer("1.1.0-rc.1", "1.1.0"));
    }

    #[test]
    fn core_triple_wins() {
        assert!(is_newer("1.2.0-beta.1", "1.1.0-beta.9"));
        assert!(is_newer("1.1.1", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0-beta.2"));
        assert!(!is_newer("1.1.0", "1.1.0"));
    }

    #[test]
    fn prerelease_labels_order() {
        assert!(is_newer("1.1.0-rc.1", "1.1.0-beta.9")); // rc > beta
        assert!(is_newer("1.1.0-beta.2", "1.1.0-alpha.9"));
        assert!(!is_newer("1.1.0-alpha.1", "1.1.0-beta.9"));
        assert!(is_newer("1.1.0-beta.2", "1.1.0-beta")); // 缺段 = 更早
    }
}
