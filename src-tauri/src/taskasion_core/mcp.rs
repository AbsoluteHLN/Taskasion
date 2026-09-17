//! MCP 服务(stdio)——Agent 管理 Taskasion 任务的标准入口。
//!
//! 用 serde_json 手写 newline-delimited JSON-RPC 2.0,不再依赖 `pip install mcp`:
//! - initialize → 回显客户端 protocolVersion + serverInfo{name:"taskasion"};
//! - tools/list → 10 个工具(描述与原 Python FastMCP 版逐字一致);
//! - tools/call → 结果包成 {content:[{type:"text",text:<JSON>}],isError};
//! - 通知(无 id)不回包;未知方法 → -32601;日志只走 stderr。

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

use super::models::Task;
use super::rest::Core;
use super::store::CoreError;
use super::VERSION;

/// 运行 MCP stdio 服务(阻塞当前线程,读到 EOF 退出)。
pub fn run(data_dir: &Path) -> Result<(), CoreError> {
    let core = Core::open(data_dir);
    eprintln!("taskasion-mcp {} ready  data={}", VERSION, core.data_dir.display());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else { continue };
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        match method {
            "initialize" => {
                let Some(id) = id else { continue };
                let result = json!({
                    "protocolVersion": params
                        .get("protocolVersion")
                        .cloned()
                        .unwrap_or(json!("2024-11-05")),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "taskasion", "version": VERSION },
                });
                write_result(&mut out, &id, result);
            }
            "ping" => {
                if let Some(id) = id {
                    write_result(&mut out, &id, json!({}));
                }
            }
            "tools/list" => {
                let Some(id) = id else { continue };
                write_result(&mut out, &id, json!({ "tools": tool_defs() }));
            }
            "tools/call" => {
                let Some(id) = id else { continue };
                let (result, is_error) = match tools_call(&core, &params) {
                    Ok(v) => (serde_json::to_string(&v).unwrap_or_default(), false),
                    Err(err) => (err.to_string(), true),
                };
                write_result(
                    &mut out,
                    &id,
                    json!({
                        "content": [{ "type": "text", "text": result }],
                        "isError": is_error,
                    }),
                );
            }
            _ => {
                // 通知(no id)一律忽略;带 id 的未知方法按 JSON-RPC 规范报错
                if let Some(id) = id {
                    write_error(&mut out, &id, -32601, "Method not found");
                }
            }
        }
    }
    Ok(())
}

fn write_result(out: &mut impl Write, id: &Value, result: Value) {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let _ = writeln!(out, "{envelope}");
    let _ = out.flush();
}

fn write_error(out: &mut impl Write, id: &Value, code: i64, message: &str) {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
    let _ = writeln!(out, "{envelope}");
    let _ = out.flush();
}

// ---------------------------------------------------------------- 工具定义

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        },
    })
}

fn str_prop() -> Value {
    json!({ "type": "string" })
}

fn tags_prop() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

/// 10 个工具的描述与原 Python FastMCP 版逐字一致。
fn tool_defs() -> Vec<Value> {
    vec![
        tool(
            "task_add",
            "新增待办。due 格式 YYYY-MM-DD(可空);priority 为 p1/p2/p3(可空);tags 为标签列表(可空);note 为备注(可空)。",
            json!({
                "title": str_prop(),
                "due": str_prop(),
                "priority": str_prop(),
                "tags": tags_prop(),
                "note": str_prop(),
            }),
            &["title"],
        ),
        tool(
            "task_list",
            "列出任务。status: todo/done/all,默认 todo;tag 可选过滤。",
            json!({ "status": str_prop(), "tag": str_prop() }),
            &[],
        ),
        tool(
            "task_update",
            "更新任务。只传需要修改的字段,未传字段保持不变;清空 due/note 请传 'none'。",
            json!({
                "task_id": str_prop(),
                "title": str_prop(),
                "due": str_prop(),
                "priority": str_prop(),
                "tags": tags_prop(),
                "note": str_prop(),
            }),
            &["task_id"],
        ),
        tool("task_complete", "勾选完成任务。", json!({ "task_id": str_prop() }), &["task_id"]),
        tool("task_reopen", "把已完成任务回退为待办。", json!({ "task_id": str_prop() }), &["task_id"]),
        tool("task_delete", "删除任务。", json!({ "task_id": str_prop() }), &["task_id"]),
        tool("task_plan_today", "今日规划:返回 {today, overdue, today_tasks, next}。", json!({}), &[]),
        tool("goal_add", "新建目标(长期意向)。返回目标 dict(含 progress 进度)。", json!({ "title": str_prop() }), &["title"]),
        tool("goal_list", "列出目标。status: todo/done/all,默认 todo。", json!({ "status": str_prop() }), &[]),
        tool(
            "goal_link_task",
            "把任务关联到目标(给任务打 goal:<id> 标签)。",
            json!({ "goal_id": str_prop(), "task_id": str_prop() }),
            &["goal_id", "task_id"],
        ),
    ]
}

fn arg<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get("arguments").and_then(|a| a.get(name)).and_then(Value::as_str)
}

fn task_value(t: &Task) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// tools/call 分发;Err → isError 结果。工具语义与 Python 版逐条一致(空串=未传)。
fn tools_call(core: &Core, params: &Value) -> Result<Value, CoreError> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let tags_arg = || -> Option<Vec<String>> {
        args.get("tags").and_then(Value::as_array).map(|arr| {
            arr.iter().filter_map(Value::as_str).map(str::to_string).collect()
        })
    };
    match name {
        "task_add" => {
            let task = core.store.add(
                arg(params, "title").unwrap_or(""),
                arg(params, "due").filter(|s| !s.is_empty()),
                arg(params, "priority").filter(|s| !s.is_empty()),
                tags_arg(),
                "agent:mcp",
                arg(params, "note").filter(|s| !s.is_empty()),
            )?;
            Ok(serde_json::to_value(&task).unwrap_or(Value::Null))
        }
        "task_list" => {
            let status = arg(params, "status").unwrap_or("todo");
            let tag = arg(params, "tag").filter(|t| !t.is_empty());
            let list: Vec<Value> =
                core.store.list(status, tag).iter().map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).collect();
            Ok(Value::Array(list))
        }
        "task_update" => {
            let task_id = arg(params, "task_id").unwrap_or("").to_string();
            let mut fields = serde_json::Map::new();
            let arg_owned = |name: &str| arg(params, name).map(str::to_string);
            if let Some(v) = arg_owned("title") {
                if !v.is_empty() {
                    fields.insert("title".into(), json!(v));
                }
            }
            if let Some(v) = arg_owned("due") {
                if !v.is_empty() {
                    fields.insert("due".into(), if v == "none" { Value::Null } else { json!(v) });
                }
            }
            if let Some(v) = arg_owned("priority") {
                if !v.is_empty() {
                    fields.insert("priority".into(), json!(v));
                }
            }
            if let Some(v) = args.get("tags") {
                if v.as_array().is_some_and(|a| !a.is_empty()) {
                    fields.insert("tags".into(), v.clone());
                }
            }
            if let Some(v) = arg_owned("note") {
                if !v.is_empty() {
                    fields.insert("note".into(), if v == "none" { Value::Null } else { json!(v) });
                }
            }
            let task = core.store.update(&task_id, &fields, "agent:mcp")?;
            Ok(task_value(&task))
        }
        "task_complete" => {
            let task = core.store.set_done(arg(params, "task_id").unwrap_or(""), true, "agent:mcp")?;
            Ok(task_value(&task))
        }
        "task_reopen" => {
            let task = core.store.set_done(arg(params, "task_id").unwrap_or(""), false, "agent:mcp")?;
            Ok(task_value(&task))
        }
        "task_delete" => {
            core.store.delete(arg(params, "task_id").unwrap_or(""), "agent:mcp")?;
            Ok(json!({ "ok": true }))
        }
        "task_plan_today" => Ok(core.store.plan_today()),
        "goal_add" => {
            let goal = core.goals.add(arg(params, "title").unwrap_or(""), "agent:mcp")?;
            Ok(goal_value(&goal))
        }
        "goal_list" => {
            let status = arg(params, "status").unwrap_or("todo");
            let list: Vec<Value> = core.goals.list(status).iter().map(goal_value).collect();
            Ok(Value::Array(list))
        }
        "goal_link_task" => {
            let goal_id = arg(params, "goal_id").unwrap_or("");
            let task_id = arg(params, "task_id").unwrap_or("").to_string();
            let mut tags: Vec<String> = core
                .store
                .get(&task_id)
                .map(|t| t.tags.clone())
                .unwrap_or_default();
            let tag = format!("goal:{goal_id}");
            if !tags.contains(&tag) {
                tags.push(tag);
            }
            let fields = json!({ "tags": tags });
            let task = core.store.update(&task_id, fields.as_object().unwrap(), "agent:mcp")?;
            Ok(task_value(&task))
        }
        _ => Err(CoreError::Invalid(format!("Unknown tool: {name}"))),
    }
}

fn goal_value(goal: &Task) -> Value {
    serde_json::to_value(goal).unwrap_or(Value::Null)
}
