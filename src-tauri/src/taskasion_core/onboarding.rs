//! Agent 自助接入:onboarding 文本与 AGENTS.md 模板。
//!
//! 设计目标:用户只需对 Agent 说"某目录下有 Taskasion",Agent 运行
//! `Taskasion.exe onboarding`(或直接读安装目录里生成的 AGENTS.md)即可
//! 自行完成接入 —— 全程无需人工写任何客户端配置。
//! 本模块只做纯文本生成(便于单测);端口探测与落盘由 main.rs 的 CLI 层完成。

use std::path::Path;

/// 探测桌面 widget 是否正在运行(127.0.0.1:14411 可连即认为在)。
pub fn widget_port_alive() -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], 14411)), Duration::from_millis(300))
        .is_ok()
}

/// CLI `onboarding` 的完整输出(人读 + Agent 读两用)。
pub fn onboarding_text(exe: &Path, data_dir: &Path, widget_running: bool, version: &str) -> String {
    let exe = exe.display();
    let data = data_dir.display();
    let widget_line = if widget_running {
        "运行中 —— REST 已在 127.0.0.1:14411 服务(方式二立即可用)"
    } else {
        "未运行 —— 可用方式一以 MCP 子进程接入,或先启动桌面端"
    };
    format!(
        "\
Taskasion {version} — Agent 接入信息
==================================================
可执行文件: {exe}
数据目录(唯一真相源): {data}   (todo.md / goals.md / audit.jsonl)
桌面 widget: {widget_line}

方式一 · MCP(推荐,无需任何客户端配置):
  以子进程运行: \"{exe}\" mcp
  env: TASKASION_DATA_DIR={data}
  协议: stdio 换行分隔 JSON-RPC 2.0 —— 先发 initialize(clientInfo.name 填你的名字,
  审计可区分),再用 tools/list / capabilities 工具获取全部工具与约定(自述,无需读文档)。

方式二 · REST(widget 运行时):
  基址 http://127.0.0.1:14411/api/v1 ,请求头 X-Taskasion-Actor: agent:<你的名字>
  GET /api/v1/capabilities 返回与 MCP capabilities 工具相同的自述。

审计:所有变更都会以你的 actor 记入 {data}\\audit.jsonl;删除类操作不可恢复。
提示:把本输出或同目录的 AGENTS.md 直接交给 Agent 即可完成接入;
`\"{exe}\" onboarding --write` 可(重新)生成 AGENTS.md。
"
    )
}

/// 写进安装目录的 AGENTS.md(主流 Agent 客户端会自动读取该文件)。
pub fn agents_md(exe: &Path, data_dir: &Path) -> String {
    let exe = exe.display();
    let data = data_dir.display();
    format!(
        "\
# AGENTS.md — Taskasion 自动接入

本目录是一个 Taskasion 安装(本地优先的 todo/goal 工具,真相源为 Markdown 文件)。
当用户要求管理任务/目标时,按下述方式**自行接入,不需要用户修改任何配置**。

- 数据目录(唯一真相源): {data}
  内含 todo.md / goals.md / audit.jsonl;外部(包括你)直接编辑会被 Core 自动感知。
- 方式一 · MCP(推荐): 以子进程运行 `\"{exe}\" mcp`,env `TASKASION_DATA_DIR={data}`。
  stdio 换行分隔 JSON-RPC 2.0:先发 `initialize`(`clientInfo.name` 填你的名字,审计可区分),
  再 `tools/list`;工具语义、null/\"none\" 清空约定等由 `capabilities` 工具自述。
- 方式二 · REST(桌面 widget 运行时): 基址 `http://127.0.0.1:14411/api/v1`,
  请求头 `X-Taskasion-Actor: agent:<你的名字>`;`GET /api/v1/capabilities` 与 MCP 同一份自述。
- 红线:删除类操作不可恢复;所有变更以你的 actor 记入 `data\\audit.jsonl`。

(本文件由 `\"{exe}\" onboarding --write` 生成;安装目录变化后请重新生成。)
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"D:\tools\Taskasion\Taskasion.exe";
    const DATA: &str = r"D:\tools\Taskasion\data";

    #[test]
    fn onboarding_text_contains_both_connection_paths() {
        let t = onboarding_text(Path::new(EXE), Path::new(DATA), true, "1.2.3");
        assert!(t.contains(EXE) && t.contains(DATA));
        assert!(t.contains("TASKASION_DATA_DIR="));
        assert!(t.contains("\" mcp") && t.contains("JSON-RPC 2.0"));
        assert!(t.contains("http://127.0.0.1:14411/api/v1"));
        assert!(t.contains("X-Taskasion-Actor"));
        assert!(t.contains("capabilities"));
        assert!(t.contains("1.2.3"));
        assert!(t.contains("运行中"), "widget_running=true 应写明 REST 可用");
        // 未运行时措辞变化
        let t2 = onboarding_text(Path::new(EXE), Path::new(DATA), false, "1.2.3");
        assert!(t2.contains("未运行"));
    }

    #[test]
    fn agents_md_is_self_sufficient_for_an_agent() {
        let m = agents_md(Path::new(EXE), Path::new(DATA));
        // 一份 AGENTS.md 要让 Agent 不看任何其它文档就能发起第一次调用
        for key in [EXE, DATA, "TASKASION_DATA_DIR=", "initialize", "tools/list", "capabilities", "X-Taskasion-Actor"] {
            assert!(m.contains(key), "AGENTS.md 缺少关键信息: {key}");
        }
        assert!(m.contains("--write"), "应说明再生成方式");
    }
}
