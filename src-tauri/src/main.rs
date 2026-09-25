// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod reminder;
mod taskasion_core;
mod update;

use std::sync::Arc;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

const CORE_PORT: u16 = 14411;

// 显示/隐藏悬浮窗(按可见性切换;托盘交互会抢走窗口焦点,不能用 is_focused 判断)
fn toggle_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

// 头部 ✕ = 退出整个 Taskasion(core 与壳同进程,随壳一起退出)
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

// 前端更新按钮:检查更新,结果经 update-status 事件推送
#[tauri::command]
fn check_update(app: tauri::AppHandle) {
    std::thread::spawn(move || update::check_and_emit(&app));
}

// 前端更新按钮:下载 → 换 exe → 重启
#[tauri::command]
fn apply_update(app: tauri::AppHandle) {
    update::apply(app);
}

// release 是 windows 子系统(无控制台);CLI 子命令运行时挂回父终端,让 stdout 可见
#[cfg(windows)]
fn attach_console() {
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX); // ATTACH_PARENT_PROCESS
    }
}

// CLI 子命令:taskasion serve / taskasion mcp(替代原 python -m taskasion_core …)
fn run_cli(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some("serve") => {
            let mut host = "127.0.0.1".to_string();
            let mut port = CORE_PORT;
            let mut data_dir = taskasion_core::rest::default_data_dir();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--host" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            host = v.clone();
                        }
                    }
                    "--port" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            port = v.parse().unwrap_or(14411);
                        }
                    }
                    "--data-dir" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            data_dir = std::path::PathBuf::from(v);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if let Err(e) = taskasion_core::rest::serve(&host, port, &data_dir, 0) {
                eprintln!("taskasion-core 启动失败: {e}");
                std::process::exit(1);
            }
            true
        }
        Some("onboarding") => {
            // Agent 自助接入:打印接入信息;--write 额外在安装目录生成 AGENTS.md。
            // 用户只需告知 Agent"某目录下有 Taskasion",Agent 跑这条命令即可自行接入。
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("Taskasion.exe"));
            let dir = exe.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            let data = dir.join("data");
            let alive = taskasion_core::onboarding::widget_port_alive();
            print!(
                "{}",
                taskasion_core::onboarding::onboarding_text(
                    &exe,
                    &data,
                    alive,
                    taskasion_core::VERSION
                )
            );
            if args.iter().any(|a| a == "--write") {
                let path = dir.join("AGENTS.md");
                match std::fs::write(&path, taskasion_core::onboarding::agents_md(&exe, &data)) {
                    Ok(()) => println!("\n已生成 {}", path.display()),
                    Err(e) => eprintln!("写入 {} 失败: {e}", path.display()),
                }
            }
            true
        }
        Some("mcp") => {
            let mut data_dir = taskasion_core::rest::default_data_dir();
            // --actor 显式指定审计身份;不给(空串)则由会话按 env / clientInfo.name 派生
            let mut actor = String::new();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--data-dir" => {
                        if let Some(v) = args.get(i + 1) {
                            data_dir = std::path::PathBuf::from(v);
                        }
                        i += 2;
                    }
                    "--actor" => {
                        if let Some(v) = args.get(i + 1) {
                            if !v.is_empty() {
                                actor = v.clone();
                            }
                        }
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            if let Err(e) = taskasion_core::mcp::run(&data_dir, &actor) {
                eprintln!("taskasion-mcp 错误: {e}");
                std::process::exit(1);
            }
            true
        }
        _ => false,
    }
}

fn main() {
    // CLI 子命令(serve / mcp);无参数或 GUI 模式继续
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        attach_console();
    }
    if run_cli(&args) {
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![quit_app, check_update, apply_update])
        .plugin(
            ShortcutBuilder::new()
                .with_shortcuts(["ctrl+shift+space"])
                .expect("failed to register global shortcut")
                .with_handler(|app, _shortcut, event| {
                    // 全局快捷键:显示/隐藏悬浮窗
                    if event.state() == ShortcutState::Pressed {
                        toggle_main(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            // 内置 core:与壳同进程的线程,数据目录 = exe 同级 data\(与原绿色模式一致)
            let data_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("data")))
                .unwrap_or_else(taskasion_core::rest::default_data_dir);

            // REST / MCP / 提醒调度共用同一个 Core 实例:一份内存态、一个审计器。
            let core = Arc::new(taskasion_core::rest::Core::open(&data_dir));
            {
                let core = core.clone();
                let data_dir = data_dir.clone();
                std::thread::spawn(move || {
                    // 绑定失败按 250ms 重试约 10s(更新换 exe 后等旧实例释放端口);
                    // 仍失败则放弃,前端会落到已存在的其它实例 core 上(与旧语义一致)
                    if let Err(e) =
                        taskasion_core::rest::serve_shared(core, "127.0.0.1", CORE_PORT, 40)
                    {
                        eprintln!("taskasion-core 启动失败: {e}  data={}", data_dir.display());
                    }
                });
            }

            // 提醒调度:只读真相源,到点响一声并把 reminder-fired 推给前端高亮
            reminder::spawn(core, app.handle().clone());

            // 清理上次更新遗留的 taskasion.exe.old,并安排启动后静默检查更新
            update::cleanup_old();
            update::schedule_auto_check(app.handle().clone());

            // 启动时停靠到主屏右上角
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = win.current_monitor() {
                    let ms = monitor.size();
                    let ws = win.outer_size().unwrap_or_default();
                    let x = ms.width.saturating_sub(ws.width).saturating_sub(28) as i32;
                    let y = 110i32;
                    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }

            // 系统托盘:出现在任务栏右侧图标区,提供"方便关闭"的入口
            let toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏", true, None::<&str>)?;
            let update = MenuItem::with_id(app, "update", "检查更新", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 Taskasion", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &update, &quit])?;
            TrayIconBuilder::with_id("taskasion-tray")
                .icon(app.default_window_icon().expect("missing window icon").clone())
                .tooltip("Taskasion — Ctrl+Shift+Space 显示/隐藏")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_main(app),
                    "update" => {
                        // 检查更新并把窗口带出来,结果体现在头部更新按钮上
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                        let handle = app.clone();
                        std::thread::spawn(move || update::check_and_emit(&handle));
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // 左键单击托盘图标 = 显示/隐藏悬浮窗
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // core 已内置同进程,无需回收子进程
        });
}
