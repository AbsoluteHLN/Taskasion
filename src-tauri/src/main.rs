// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::TcpStream;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

const CORE_PORT: u16 = 14411;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

// 绿色模式:端口空闲时用同目录绿色 runtime 拉起本地 core(数据在同级 data\)。
// 已有 core 在跑(开发源码实例或上次残留)则直接复用,不重复拉起。
fn spawn_core_if_free(app: &tauri::AppHandle) {
    let child: Option<Child> = (|| {
        if TcpStream::connect(("127.0.0.1", CORE_PORT)).is_ok() {
            return None;
        }
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let python = exe_dir.join("runtime").join("python.exe");
        if !python.exists() {
            return None; // 开发目录没有绿色 runtime,由外部手动起 core
        }
        Command::new(&python)
            .args(["-m", "taskasion_core", "serve", "--host", "127.0.0.1", "--port", "14411"])
            .arg("--data-dir")
            .arg(exe_dir.join("data"))
            .env("PYTHONPATH", &exe_dir)
            .current_dir(&exe_dir)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .ok()
    })();
    app.manage(Mutex::new(child));
}

fn main() {
    tauri::Builder::default()
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
            spawn_core_if_free(app.handle());

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
            let quit = MenuItem::with_id(app, "quit", "退出 Taskasion", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            TrayIconBuilder::with_id("taskasion-tray")
                .icon(app.default_window_icon().expect("missing window icon").clone())
                .tooltip("Taskasion — Ctrl+Shift+Space 显示/隐藏")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_main(app),
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
        .run(|_app, event| {
            // 退出时回收本壳拉起的 core 子进程(壳被强杀则 core 残留,
            // 下次启动因端口占用会直接复用,数据无状态,安全)
            if let tauri::RunEvent::Exit = event {
                if let Some(mut child) = _app
                    .state::<Mutex<Option<Child>>>()
                    .lock()
                    .ok()
                    .and_then(|mut guard| guard.take())
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        });
}
