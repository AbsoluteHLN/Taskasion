//! MCP 服务(stdio)——Agent 管理 Taskasion 任务的标准入口。
//!
//! 用 serde_json 手写 newline-delimited JSON-RPC 2.0,不再依赖 `pip install mcp`:
//! - initialize → 回显客户端 protocolVersion + serverInfo{name:"taskasion"};
//! - tools/list → 工具清单;
//! - tools/call → 结果包成 {content:[{type:"text",text:<JSON>}],isError};
//! - 通知(无 id)不回包;未知方法 → -32601;日志只走 stderr。
//!
//! **与 REST 同源**:所有工具直接在同一个 `Core`(TaskStore/GoalStore/领域方法)上执行,
//! 不存在"REST 一套、MCP 一套"的双实现。字段与 Integration API v1 对齐,
//! 同时保持旧调用兼容(空串 = 未传;`"none"` = 清空)。

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

use super::models::Task;
use super::rest::{capabilities, goal_dict, Core};
use super::store::CoreError;
use super::VERSION;

/// 运行 MCP stdio 服务(阻塞当前线程,读到 EOF 退出)。
///
/// `actor` 由 `Taskasion.exe mcp --actor agent:codex` 指定,缺省 `agent:mcp`,
/// 只用于审计留痕 —— 让"谁改的"在人与多个 Agent 并存时仍然可分辨。
pub fn run(data_dir: &Path, actor: &str) -> Result<(), CoreError> {
    let core = Core::open(data_dir);
    eprintln!(
        "taskasion-mcp {} ready  actor={}  data={}",
        VERSION,
        actor,
        core.data_dir.display()
    );
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
                let (result, is_error) = match tools_call(&core, &params, actor) {
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

/// 可清空字段:`null`(新接口)或 `"none"`(旧约定)表示清空。
fn nullable_str_prop() -> Value {
    json!({ "type": ["string", "null"], "description": "传 null(或旧写法 \"none\")表示清空" })
}

fn tags_prop() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

/// 工具清单。前 10 个与原 Python FastMCP 版逐字一致(保持向后兼容),
/// 之后是与 REST `/api/v1` 对齐补齐的目标类工具与能力自述。
fn tool_defs() -> Vec<Value> {
    vec![
        tool(
            "task_add",
            "新增待办。due 格式 YYYY-MM-DD(可空);remind_time 格式 HH:MM(可空,仅在该时间提醒一次);priority 为 p1/p2/p3(可空);tags 为标签列表(可空);note 为备注(可空)。",
            json!({
                "title": str_prop(),
                "due": str_prop(),
                "remind_time": str_prop(),
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
            "更新任务。只传需要修改的字段,未传字段保持不变;清空 due/remind_time/note 请传 null(旧的 'none' 同样接受)。",
            json!({
                "task_id": str_prop(),
                "title": str_prop(),
                "due": nullable_str_prop(),
                "remind_time": nullable_str_prop(),
                "priority": nullable_str_prop(),
                "tags": tags_prop(),
                "note": nullable_str_prop(),
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
        tool("goal_update", "重命名目标。", json!({ "goal_id": str_prop(), "title": str_prop() }), &["goal_id", "title"]),
        tool("goal_complete", "把目标标记为达成。", json!({ "goal_id": str_prop() }), &["goal_id"]),
        tool("goal_reopen", "把目标回退为进行中。", json!({ "goal_id": str_prop() }), &["goal_id"]),
        tool("goal_delete", "删除目标(不会删除其关联任务)。", json!({ "goal_id": str_prop() }), &["goal_id"]),
        tool(
            "capabilities",
            "返回 Integration API v1 的能力自述(REST/MCP 入口、actor 约定、Task 字段、null 语义、提醒模型)。",
            json!({}),
            &[],
        ),
    ]
}

fn arg<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get("arguments").and_then(|a| a.get(name)).and_then(Value::as_str)
}

/// 可清空字段的取值:
/// - 传 `null` → `Some(Null)`,清空;
/// - 传 `"none"` → `Some(Null)`,清空(保留旧版约定);
/// - 传 `""` / 未传 → `None`,不改动(旧版"空串=未传"语义不变)。
fn clearable_arg(params: &Value, name: &str) -> Option<Value> {
    match params.get("arguments").and_then(|a| a.get(name))? {
        Value::Null => Some(Value::Null),
        Value::String(s) if s == "none" => Some(Value::Null),
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(json!(s)),
        other => Some(other.clone()),
    }
}

fn task_value(t: &Task) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// tools/call 分发;Err → isError 结果。与 REST 走同一批 Core 领域方法。
fn tools_call(core: &Core, params: &Value, actor: &str) -> Result<Value, CoreError> {
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
                arg(params, "remind_time").filter(|s| !s.is_empty()),
                arg(params, "priority").filter(|s| !s.is_empty()),
                tags_arg(),
                actor,
                arg(params, "note").filter(|s| !s.is_empty()),
            )?;
            Ok(task_value(&task))
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
            // 可清空字段:null / "none" → 清空;"" 或未传 → 不改动。
            for key in ["due", "remind_time", "priority", "note"] {
                if let Some(v) = clearable_arg(params, key) {
                    fields.insert(key.into(), v);
                }
            }
            if let Some(v) = args.get("tags") {
                if v.as_array().is_some_and(|a| !a.is_empty()) {
                    fields.insert("tags".into(), v.clone());
                }
            }
            let task = core.store.update(&task_id, &fields, actor)?;
            Ok(task_value(&task))
        }
        "task_complete" => {
            let task = core.store.set_done(arg(params, "task_id").unwrap_or(""), true, actor)?;
            Ok(task_value(&task))
        }
        "task_reopen" => {
            let task = core.store.set_done(arg(params, "task_id").unwrap_or(""), false, actor)?;
            Ok(task_value(&task))
        }
        "task_delete" => {
            core.store.delete(arg(params, "task_id").unwrap_or(""), actor)?;
            Ok(json!({ "ok": true }))
        }
        "task_plan_today" => Ok(core.store.plan_today()),
        "goal_add" => {
            let goal = core.goals.add(arg(params, "title").unwrap_or(""), actor)?;
            Ok(goal_dict(core, &goal))
        }
        "goal_list" => {
            let status = arg(params, "status").unwrap_or("todo");
            let list: Vec<Value> = core.goals.list(status).iter().map(|g| goal_dict(core, g)).collect();
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
            let task = core.store.update(&task_id, fields.as_object().unwrap(), actor)?;
            Ok(task_value(&task))
        }
        "goal_update" => {
            let goal = core.goals.rename(
                arg(params, "goal_id").unwrap_or(""),
                arg(params, "title").unwrap_or(""),
                actor,
            )?;
            Ok(goal_dict(core, &goal))
        }
        "goal_complete" => {
            let goal = core.goals.set_done(arg(params, "goal_id").unwrap_or(""), true, actor)?;
            Ok(goal_dict(core, &goal))
        }
        "goal_reopen" => {
            let goal = core.goals.set_done(arg(params, "goal_id").unwrap_or(""), false, actor)?;
            Ok(goal_dict(core, &goal))
        }
        "goal_delete" => {
            core.goals.delete(arg(params, "goal_id").unwrap_or(""), actor)?;
            Ok(json!({ "ok": true }))
        }
        "capabilities" => Ok(capabilities(core)),
        _ => Err(CoreError::Invalid(format!("Unknown tool: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taskasion-mcp-test-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call(core: &Core, tool: &str, args: Value) -> Result<Value, CoreError> {
        tools_call(core, &json!({ "name": tool, "arguments": args }), "agent:mcp")
    }

    fn call_as(core: &Core, tool: &str, args: Value, actor: &str) -> Result<Value, CoreError> {
        tools_call(core, &json!({ "name": tool, "arguments": args }), actor)
    }

    #[test]
    fn legacy_tools_are_all_still_present_and_names_are_unique() {
        let defs = tool_defs();
        let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
        // Python FastMCP 版的 10 个工具名一个都不能少(老客户端写死了这些名字)
        for legacy in [
            "task_add",
            "task_list",
            "task_update",
            "task_complete",
            "task_reopen",
            "task_delete",
            "task_plan_today",
            "goal_add",
            "goal_list",
            "goal_link_task",
        ] {
            assert!(names.contains(&legacy), "缺少历史工具 {legacy}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "工具名不能重复");

        // 每个工具的 schema 都是合法对象,且 required 字段都在 properties 里
        for d in &defs {
            let schema = &d["inputSchema"];
            assert_eq!(schema["type"], json!("object"));
            let props = schema["properties"].as_object().unwrap();
            for r in schema["required"].as_array().unwrap() {
                assert!(props.contains_key(r.as_str().unwrap()), "{} 的 required 字段缺 schema", d["name"]);
            }
        }
    }

    #[test]
    fn task_add_and_update_carry_remind_time() {
        let dir = tmp_dir("remind");
        let core = Core::open(&dir);
        let added = call(
            &core,
            "task_add",
            json!({ "title": "开会", "due": "2026-09-24", "remind_time": "14:30", "tags": ["work"] }),
        )
        .unwrap();
        assert_eq!(added["remind_time"], json!("14:30"));
        assert_eq!(added["tags"], json!(["work"]), "保留标签不得外泄");
        let id = added["id"].as_str().unwrap().to_string();

        // 改时刻
        let moved = call(&core, "task_update", json!({ "task_id": id, "remind_time": "16:05" })).unwrap();
        assert_eq!(moved["remind_time"], json!("16:05"));

        // 清空:新写法 null 与旧写法 "none" 等价
        let cleared = call(&core, "task_update", json!({ "task_id": id, "remind_time": "none" })).unwrap();
        assert_eq!(cleared["remind_time"], Value::Null);
        let set_again = call(&core, "task_update", json!({ "task_id": id, "remind_time": "08:00" })).unwrap();
        assert_eq!(set_again["remind_time"], json!("08:00"));
        let cleared2 = call(&core, "task_update", json!({ "task_id": id, "remind_time": null })).unwrap();
        assert_eq!(cleared2["remind_time"], Value::Null);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_string_still_means_not_provided_for_legacy_clients() {
        let dir = tmp_dir("empty-string");
        let core = Core::open(&dir);
        let added = call(&core, "task_add", json!({ "title": "旧客户端", "note": "原文", "due": "2026-09-24" })).unwrap();
        let id = added["id"].as_str().unwrap().to_string();

        // 空串在 MCP 里历史语义就是"没传"(不是清空)—— 老 Agent 大量这么发
        let kept = call(&core, "task_update", json!({ "task_id": id, "note": "", "due": "" })).unwrap();
        assert_eq!(kept["note"], json!("原文"));
        assert_eq!(kept["due"], json!("2026-09-24"));

        // 要清空就用 "none"(旧)或 null(新)
        let cleared = call(&core, "task_update", json!({ "task_id": id, "note": "none" })).unwrap();
        assert_eq!(cleared["note"], Value::Null);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn actor_flag_lands_in_the_audit_trail() {
        let dir = tmp_dir("actor");
        let core = Core::open(&dir);
        let added = call_as(&core, "task_add", json!({ "title": "Agent 建的" }), "agent:codex").unwrap();
        let id = added["id"].as_str().unwrap().to_string();
        call_as(&core, "task_complete", json!({ "task_id": id }), "agent:deepseek").unwrap();

        let tail = core.audit.tail(10);
        let actors: Vec<&str> = tail.iter().filter_map(|e| e["actor"].as_str()).collect();
        assert!(actors.contains(&"agent:codex"), "add 应记在 agent:codex 名下: {tail:?}");
        assert!(actors.contains(&"agent:deepseek"), "complete 应记在 agent:deepseek 名下: {tail:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_tools_and_capabilities_match_rest() {
        let dir = tmp_dir("parity");
        let core = Core::open(&dir);
        let goal = call(&core, "goal_add", json!({ "title": "AI 建筑" })).unwrap();
        assert_eq!(goal["progress"], json!({ "total": 0, "done": 0 }), "目标必须带实时进度");
        let gid = goal["id"].as_str().unwrap().to_string();

        let task = call(&core, "task_add", json!({ "title": "打样" })).unwrap();
        let tid = task["id"].as_str().unwrap().to_string();
        call(&core, "goal_link_task", json!({ "goal_id": gid, "task_id": tid })).unwrap();
        call(&core, "task_complete", json!({ "task_id": tid })).unwrap();

        let listed = call(&core, "goal_list", json!({})).unwrap();
        assert_eq!(listed[0]["progress"], json!({ "total": 1, "done": 1 }));

        let renamed = call(&core, "goal_update", json!({ "goal_id": gid, "title": "AI 建筑 v2" })).unwrap();
        assert_eq!(renamed["title"], json!("AI 建筑 v2"));
        assert_eq!(call(&core, "goal_complete", json!({ "goal_id": gid })).unwrap()["done"], json!(true));
        assert_eq!(call(&core, "goal_reopen", json!({ "goal_id": gid })).unwrap()["done"], json!(false));
        assert_eq!(call(&core, "goal_delete", json!({ "goal_id": gid })).unwrap()["ok"], json!(true));
        assert!(call(&core, "goal_list", json!({})).unwrap().as_array().unwrap().is_empty());

        // MCP 的 capabilities 与 REST 的 /api/capabilities 是同一份描述
        let caps = call(&core, "capabilities", json!({})).unwrap();
        assert_eq!(caps["api_version"], json!(1));
        assert_eq!(caps["mcp"]["default_actor"], json!("agent:mcp"));

        // 未知工具仍然是明确的错误,而不是静默成功
        assert!(call(&core, "task_teleport", json!({})).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
