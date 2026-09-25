//! MCP 服务(stdio)——Agent 管理 Taskasion 任务的标准入口。
//!
//! 用 serde_json 手写 newline-delimited JSON-RPC 2.0,不依赖任何 SDK:
//! - initialize → 回显客户端 protocolVersion,声明 capabilities(tools/logging),
//!   附 instructions 使用说明;审计 actor 解析顺序:--actor 显式指定 >
//!   env TASKASION_MCP_ACTOR > clientInfo.name 派生(如 `agent:mcp:codex`)>
//!   缺省 `agent:mcp` —— 不同客户端在 audit.jsonl 里可分辨;
//! - tools/list → 表驱动工具注册表(含规范 annotations:readOnly/destructive/idempotent
//!   提示,客户端可据此对工具分级放行);
//! - tools/call → 结果按规范 text + structuredContent 双写(两者是同一个 JSON 值,
//!   旧客户端读 text,新客户端读结构化对象);业务失败 isError:true 且
//!   structuredContent.error 带错误码;未知工具 → 协议错误 -32602(对齐规范示例);
//! - 日志经 notifications/message 同步给客户端(stderr 保留);
//! - 通知(无 id)不回包;未知方法 → -32601。
//!
//! **与 REST 同源**:所有工具直接在同一个 `Core`(TaskStore/GoalStore/领域方法)上执行,
//! 不存在"REST 一套、MCP 一套"的双实现。字段与 Integration API v1 对齐,
//! 同时保持旧调用兼容(空串 = 未传;`"none"` = 清空)。

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::OnceLock;

use serde_json::{json, Value};

use super::models::Task;
use super::rest::{capabilities, goal_dict, Core};
use super::store::CoreError;
use super::VERSION;

const DEFAULT_ACTOR: &str = "agent:mcp";

/// initialize.instructions:告诉 Agent 本服务器的关键约定。
const INSTRUCTIONS: &str = "\
Taskasion 是本地优先的 todo/goal 系统:真相源是数据目录里的 todo.md 与 goals.md 两个 Markdown 文件,所有改动立即落盘、追加审计;人可以直接编辑文件,外部修改会被自动感知。
使用要点:
- 先用 task_list / goal_list 拿到条目 id(8 位 hex),再对具体 id 操作;capabilities 工具返回完整能力自述;
- due 格式 YYYY-MM-DD;remind_time 格式 HH:MM(需同时有 due,到点提醒一次);priority 为 p1/p2/p3;
- 更新时未传的字段保持不变;清空 due/remind_time/priority/note 传 null(或旧写法 \"none\");空串 = 未传;
- goal_id 参数(或 goal_link_task)把任务挂到目标,目标进度自动聚合;goal_unlink_task 取消关联;
- task_plan_today 返回 {today, overdue, today_tasks, next},适合作为一天的开始;
- 删除类操作(task_delete / goal_delete)不可恢复,请确认后再调用。";

// ---------------------------------------------------------------- 会话

pub struct McpSession {
    core: Core,
    /// 本会话的审计身份,initialize 时按优先级解析。
    actor: String,
    /// `--actor` 显式指定(空串 = 未指定),优先级最高。
    actor_flag: Option<String>,
    initialized: bool,
}

impl McpSession {
    pub fn new(data_dir: &Path, actor_flag: &str) -> McpSession {
        McpSession {
            core: Core::open(data_dir),
            actor: DEFAULT_ACTOR.to_string(),
            actor_flag: (!actor_flag.is_empty()).then(|| actor_flag.to_string()),
            initialized: false,
        }
    }

    /// 处理一行输入(空行/坏 JSON 静默忽略),响应与日志通知写进 out。
    pub fn handle_line(&mut self, line: &str, out: &mut impl Write) -> std::io::Result<()> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(());
        };
        self.handle_msg(&msg, out)
    }

    fn handle_msg(&mut self, msg: &Value, out: &mut impl Write) -> std::io::Result<()> {
        // 通知(无 id)不回包
        let Some(id) = msg.get("id").cloned().filter(|v| !v.is_null()) else {
            return Ok(());
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        // Err = 协议级错误(JSON-RPC error 信封);Ok = 正常 result 信封
        let reply: Result<Value, (i64, String)> = match method {
            "initialize" => {
                self.initialized = true;
                self.apply_actor(&params);
                Ok(json!({
                    "protocolVersion": params
                        .get("protocolVersion")
                        .cloned()
                        .unwrap_or(json!("2024-11-05")),
                    "capabilities": { "tools": { "listChanged": false }, "logging": {} },
                    "serverInfo": { "name": "taskasion", "version": VERSION },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_defs() })),
            "tools/call" => self.call_tool(&params),
            _ => Err((-32601, "Method not found".to_string())),
        };

        match reply {
            Ok(result) => {
                write_result(out, &id, &result)?;
                if method == "initialize" {
                    let data = format!("taskasion-mcp {VERSION} ready  actor={}", self.actor);
                    notify_message(out, "info", &data)?;
                } else if self.initialized
                    && method == "tools/call"
                    && result.get("isError").and_then(Value::as_bool).unwrap_or(false)
                {
                    let text = result["content"][0]["text"].as_str().unwrap_or("工具执行失败");
                    notify_message(out, "error", text)?;
                }
            }
            Err((code, message)) => write_error(out, &id, code, &message)?,
        }
        Ok(())
    }

    /// actor 优先级:--actor 显式 > env TASKASION_MCP_ACTOR > clientInfo.name 派生 > 缺省。
    fn apply_actor(&mut self, params: &Value) {
        if let Some(flag) = &self.actor_flag {
            self.actor = flag.clone();
            return;
        }
        if let Ok(env) = std::env::var("TASKASION_MCP_ACTOR") {
            if !env.is_empty() {
                self.actor = env;
                return;
            }
        }
        let name = params
            .get("clientInfo")
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        self.actor = match sanitize_client_name(name) {
            clean if clean.is_empty() => DEFAULT_ACTOR.to_string(),
            clean => format!("{DEFAULT_ACTOR}:{clean}"),
        };
    }

    /// tools/call:未知工具 → 协议错误 -32602;业务失败 → isError 结果(工具执行错误)。
    fn call_tool(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        if !tools().iter().any(|t| t.name == name) {
            return Err((-32602, format!("Unknown tool: {name}")));
        }
        let (payload, is_error) = match tools_call(&self.core, params, &self.actor) {
            Ok(v) => (v, false),
            Err(err) => {
                let code = match &err {
                    CoreError::NotFound(_) => "not_found",
                    CoreError::Invalid(_) => "invalid",
                    CoreError::Internal(_) => "internal",
                };
                (json!({ "error": { "code": code, "message": err.to_string() } }), true)
            }
        };
        // text 与 structuredContent 是同一个 JSON 值:旧客户端解析 text,新客户端读结构化字段
        let text = serde_json::to_string(&payload).unwrap_or_default();
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": payload,
            "isError": is_error,
        }))
    }
}

/// clientInfo.name → 审计后缀:仅保留字母数字与 -_.,其余替换为 '-',首尾裁剪,截断 32。
fn sanitize_client_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .take(32)
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ---------------------------------------------------------------- stdio 输出

fn write_result(out: &mut impl Write, id: &Value, result: &Value) -> std::io::Result<()> {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(out, "{envelope}")?;
    out.flush()
}

fn write_error(out: &mut impl Write, id: &Value, code: i64, message: &str) -> std::io::Result<()> {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
    writeln!(out, "{envelope}")?;
    out.flush()
}

/// notifications/message:日志同步给客户端(仅 initialize 之后发送)。
fn notify_message(out: &mut impl Write, level: &str, data: &str) -> std::io::Result<()> {
    let note = json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": { "level": level, "data": data },
    });
    writeln!(out, "{note}")?;
    out.flush()
}

/// 运行 MCP stdio 服务(阻塞当前线程,读到 EOF 退出)。
///
/// `actor_flag` 来自 `Taskasion.exe mcp --actor …`,空串 = 未指定
/// (此时按 env / clientInfo.name 派生,详见 [`McpSession::apply_actor`])。
pub fn run(data_dir: &Path, actor_flag: &str) -> Result<(), CoreError> {
    let mut session = McpSession::new(data_dir, actor_flag);
    eprintln!("taskasion-mcp {} ready  data={}", VERSION, session.core.data_dir.display());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        // 客户端关掉 stdout(进程退出)就停止服务,不把 IO 错误伪装成 Core 错误
        if session.handle_line(&line, &mut out).is_err() {
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 工具注册表

type ToolFn = fn(&Core, &str, &Value) -> Result<Value, CoreError>;

struct ToolDef {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    /// 规范 annotations:客户端据此对只读/危险操作分级放行。
    annotations: Value,
    handler: ToolFn,
}

/// annotations 简写;openWorldHint 恒为 false(纯本地文件操作,无外部副作用)。
fn ann(read_only: bool, destructive: bool, idempotent: bool) -> Value {
    json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false,
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

/// 工具注册表:单一事实来源 —— tools/list 的清单、tools/call 的分发、协议层的
/// "未知工具"判断都查这一张表,不会漂移。前 10 个与原 Python FastMCP 版逐字一致
/// (保持向后兼容),之后是与 Integration API v1 对齐补齐的工具。
fn tools() -> &'static [ToolDef] {
    static TOOLS: OnceLock<Vec<ToolDef>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        vec![
            ToolDef {
                name: "task_add",
                description: "新增待办。due 格式 YYYY-MM-DD(可空);remind_time 格式 HH:MM(可空,需同时有 due,到点提醒一次);priority 为 p1/p2/p3(可空);tags 为标签列表(可空);note 为备注(可空);goal_id 传目标 id 则创建即关联该目标(可空)。",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "title": str_prop(),
                        "due": str_prop(),
                        "remind_time": str_prop(),
                        "priority": str_prop(),
                        "tags": tags_prop(),
                        "note": str_prop(),
                        "goal_id": str_prop(),
                    },
                    "required": ["title"],
                }),
                annotations: ann(false, false, false),
                handler: tool_task_add,
            },
            ToolDef {
                name: "task_list",
                description: "列出任务。status: todo/done/all,默认 todo;tag 按标签过滤;query 按标题/备注子串过滤(大小写不敏感,可空)。",
                input_schema: json!({
                    "type": "object",
                    "properties": { "status": str_prop(), "tag": str_prop(), "query": str_prop() },
                    "required": [],
                }),
                annotations: ann(true, false, false),
                handler: tool_task_list,
            },
            ToolDef {
                name: "task_update",
                description: "更新任务。只传需要修改的字段,未传字段保持不变;清空 due/remind_time/priority/note 传 null(旧的 'none' 同样接受);tags 传数组则整体替换。",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "task_id": str_prop(),
                        "title": str_prop(),
                        "due": nullable_str_prop(),
                        "remind_time": nullable_str_prop(),
                        "priority": nullable_str_prop(),
                        "tags": tags_prop(),
                        "note": nullable_str_prop(),
                    },
                    "required": ["task_id"],
                }),
                annotations: ann(false, false, false),
                handler: tool_task_update,
            },
            ToolDef {
                name: "task_complete",
                description: "勾选完成任务(幂等,已完成再勾选无副作用)。",
                input_schema: json!({ "type": "object", "properties": { "task_id": str_prop() }, "required": ["task_id"] }),
                annotations: ann(false, false, true),
                handler: tool_task_complete,
            },
            ToolDef {
                name: "task_reopen",
                description: "把已完成任务回退为待办(幂等)。",
                input_schema: json!({ "type": "object", "properties": { "task_id": str_prop() }, "required": ["task_id"] }),
                annotations: ann(false, false, true),
                handler: tool_task_reopen,
            },
            ToolDef {
                name: "task_delete",
                description: "删除任务,不可恢复(目标进度自动重算)。",
                input_schema: json!({ "type": "object", "properties": { "task_id": str_prop() }, "required": ["task_id"] }),
                annotations: ann(false, true, false),
                handler: tool_task_delete,
            },
            ToolDef {
                name: "task_plan_today",
                description: "今日规划:返回 {today, overdue, today_tasks, next}。",
                input_schema: json!({ "type": "object", "properties": {}, "required": [] }),
                annotations: ann(true, false, false),
                handler: tool_task_plan_today,
            },
            ToolDef {
                name: "goal_add",
                description: "新建目标(长期意向)。返回目标 dict(含 progress 进度)。",
                input_schema: json!({ "type": "object", "properties": { "title": str_prop() }, "required": ["title"] }),
                annotations: ann(false, false, false),
                handler: tool_goal_add,
            },
            ToolDef {
                name: "goal_list",
                description: "列出目标。status: todo/done/all,默认 todo。",
                input_schema: json!({ "type": "object", "properties": { "status": str_prop() }, "required": [] }),
                annotations: ann(true, false, false),
                handler: tool_goal_list,
            },
            ToolDef {
                name: "goal_update",
                description: "重命名目标。",
                input_schema: json!({
                    "type": "object",
                    "properties": { "goal_id": str_prop(), "title": str_prop() },
                    "required": ["goal_id", "title"],
                }),
                annotations: ann(false, false, false),
                handler: tool_goal_update,
            },
            ToolDef {
                name: "goal_complete",
                description: "把目标标记为达成(幂等)。",
                input_schema: json!({ "type": "object", "properties": { "goal_id": str_prop() }, "required": ["goal_id"] }),
                annotations: ann(false, false, true),
                handler: tool_goal_complete,
            },
            ToolDef {
                name: "goal_reopen",
                description: "把目标回退为进行中(幂等)。",
                input_schema: json!({ "type": "object", "properties": { "goal_id": str_prop() }, "required": ["goal_id"] }),
                annotations: ann(false, false, true),
                handler: tool_goal_reopen,
            },
            ToolDef {
                name: "goal_delete",
                description: "删除目标,不可恢复(不会删除其关联任务)。",
                input_schema: json!({ "type": "object", "properties": { "goal_id": str_prop() }, "required": ["goal_id"] }),
                annotations: ann(false, true, false),
                handler: tool_goal_delete,
            },
            ToolDef {
                name: "goal_link_task",
                description: "把任务关联到目标(给任务追加 goal:<id> 标签,已关联则幂等)。",
                input_schema: json!({
                    "type": "object",
                    "properties": { "goal_id": str_prop(), "task_id": str_prop() },
                    "required": ["goal_id", "task_id"],
                }),
                annotations: ann(false, false, true),
                handler: tool_goal_link_task,
            },
            ToolDef {
                name: "goal_unlink_task",
                description: "取消任务与目标的关联(移除 goal:<id> 标签;本来就没关联则幂等)。",
                input_schema: json!({
                    "type": "object",
                    "properties": { "goal_id": str_prop(), "task_id": str_prop() },
                    "required": ["goal_id", "task_id"],
                }),
                annotations: ann(false, false, true),
                handler: tool_goal_unlink_task,
            },
            ToolDef {
                name: "capabilities",
                description: "返回 Integration API v1 的能力自述(REST/MCP 入口、actor 约定、Task 字段、null 语义、提醒模型)。",
                input_schema: json!({ "type": "object", "properties": {}, "required": [] }),
                annotations: ann(true, false, false),
                handler: tool_capabilities,
            },
        ]
    })
}

/// tools/list 的输出(注册表 → 规范 JSON,带 annotations)。
fn tool_defs() -> Vec<Value> {
    tools()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": t.annotations,
            })
        })
        .collect()
}

/// tools/call 业务入口(协议无关,测试直接调用):未知工具/业务失败都返回 Err。
/// 协议层(会话)会先把"未知工具"翻译成 -32602,再把这里的 Err 包成 isError 结果。
fn tools_call(core: &Core, params: &Value, actor: &str) -> Result<Value, CoreError> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let def = tools()
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| CoreError::Invalid(format!("Unknown tool: {name}")))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    (def.handler)(core, actor, &args)
}

// ---------------------------------------------------------------- 参数工具

/// 读字符串参数;空串视为未传(MCP 历史语义:老 Agent 大量用空串表示"没填")。
fn arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn tags_arg(args: &Value) -> Option<Vec<String>> {
    args.get("tags")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

/// 可清空字段的取值:
/// - 传 `null` → `Some(Null)`,清空;
/// - 传 `"none"` → `Some(Null)`,清空(保留旧版约定);
/// - 传 `""` / 未传 → `None`,不改动(旧版"空串=未传"语义不变)。
fn clearable_arg(args: &Value, name: &str) -> Option<Value> {
    match args.get(name)? {
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

// ---------------------------------------------------------------- 工具实现

fn tool_task_add(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let mut tags = tags_arg(args).unwrap_or_default();
    if let Some(goal_id) = arg(args, "goal_id") {
        let tag = format!("goal:{goal_id}");
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    let task = core.store.add(
        arg(args, "title").unwrap_or(""),
        arg(args, "due").filter(|s| !s.is_empty()),
        arg(args, "remind_time").filter(|s| !s.is_empty()),
        arg(args, "priority").filter(|s| !s.is_empty()),
        (!tags.is_empty()).then_some(tags),
        actor,
        arg(args, "note").filter(|s| !s.is_empty()),
    )?;
    Ok(task_value(&task))
}

fn tool_task_list(core: &Core, _actor: &str, args: &Value) -> Result<Value, CoreError> {
    let status = arg(args, "status").unwrap_or("todo");
    let tag = arg(args, "tag");
    let query = arg(args, "query").map(str::to_lowercase);
    let list: Vec<Value> = core
        .store
        .list(status, tag)
        .into_iter()
        .filter(|t| match &query {
            None => true,
            Some(q) => {
                t.title.to_lowercase().contains(q)
                    || t.note.as_deref().map(|n| n.to_lowercase().contains(q)).unwrap_or(false)
            }
        })
        .map(|t| task_value(&t))
        .collect();
    Ok(Value::Array(list))
}

fn tool_task_update(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let task_id = arg(args, "task_id").unwrap_or("").to_string();
    let mut fields = serde_json::Map::new();
    if let Some(v) = arg(args, "title") {
        fields.insert("title".into(), json!(v));
    }
    // 可清空字段:null / "none" → 清空;"" 或未传 → 不改动。
    for key in ["due", "remind_time", "priority", "note"] {
        if let Some(v) = clearable_arg(args, key) {
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

fn tool_task_complete(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let task = core.store.set_done(arg(args, "task_id").unwrap_or(""), true, actor)?;
    Ok(task_value(&task))
}

fn tool_task_reopen(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let task = core.store.set_done(arg(args, "task_id").unwrap_or(""), false, actor)?;
    Ok(task_value(&task))
}

fn tool_task_delete(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    core.store.delete(arg(args, "task_id").unwrap_or(""), actor)?;
    Ok(json!({ "ok": true }))
}

fn tool_task_plan_today(core: &Core, _actor: &str, _args: &Value) -> Result<Value, CoreError> {
    Ok(core.store.plan_today())
}

fn tool_goal_add(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal = core.goals.add(arg(args, "title").unwrap_or(""), actor)?;
    Ok(goal_dict(core, &goal))
}

fn tool_goal_list(core: &Core, _actor: &str, args: &Value) -> Result<Value, CoreError> {
    let status = arg(args, "status").unwrap_or("todo");
    let list: Vec<Value> = core.goals.list(status).iter().map(|g| goal_dict(core, g)).collect();
    Ok(Value::Array(list))
}

fn tool_goal_update(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal = core.goals.rename(arg(args, "goal_id").unwrap_or(""), arg(args, "title").unwrap_or(""), actor)?;
    Ok(goal_dict(core, &goal))
}

fn tool_goal_complete(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal = core.goals.set_done(arg(args, "goal_id").unwrap_or(""), true, actor)?;
    Ok(goal_dict(core, &goal))
}

fn tool_goal_reopen(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal = core.goals.set_done(arg(args, "goal_id").unwrap_or(""), false, actor)?;
    Ok(goal_dict(core, &goal))
}

fn tool_goal_delete(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    core.goals.delete(arg(args, "goal_id").unwrap_or(""), actor)?;
    Ok(json!({ "ok": true }))
}

fn tool_goal_link_task(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal_id = arg(args, "goal_id").unwrap_or("");
    let task_id = arg(args, "task_id").unwrap_or("").to_string();
    let mut tags: Vec<String> = core.store.get(&task_id).map(|t| t.tags).unwrap_or_default();
    let tag = format!("goal:{goal_id}");
    if !tags.contains(&tag) {
        tags.push(tag);
    }
    let fields = json!({ "tags": tags });
    let task = core.store.update(&task_id, fields.as_object().unwrap(), actor)?;
    Ok(task_value(&task))
}

fn tool_goal_unlink_task(core: &Core, actor: &str, args: &Value) -> Result<Value, CoreError> {
    let goal_id = arg(args, "goal_id").unwrap_or("");
    let task_id = arg(args, "task_id").unwrap_or("").to_string();
    let tag = format!("goal:{goal_id}");
    let mut tags: Vec<String> = core.store.get(&task_id).map(|t| t.tags).unwrap_or_default();
    let before = tags.len();
    tags.retain(|x| x != &tag);
    // 幂等:本就没有该标签时不做无谓写回,直接回读
    let task = if tags.len() != before {
        let fields = json!({ "tags": tags });
        core.store.update(&task_id, fields.as_object().unwrap(), actor)?
    } else {
        core.store
            .get(&task_id)
            .ok_or_else(|| CoreError::NotFound(format!("任务不存在: {task_id}")))?
    };
    Ok(task_value(&task))
}

fn tool_capabilities(core: &Core, _actor: &str, _args: &Value) -> Result<Value, CoreError> {
    Ok(capabilities(core))
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

    // ---------------- 会话层辅助(协议行为测试) ----------------

    /// 调一条消息,返回写出的全部 JSON-RPC 行(响应 + 日志通知,按序)。
    fn exchange(s: &mut McpSession, msg: Value) -> Vec<Value> {
        let mut buf: Vec<u8> = Vec::new();
        s.handle_msg(&msg, &mut buf).unwrap();
        String::from_utf8(buf)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect()
    }

    /// 调一条消息,返回其中带 id 的响应(忽略日志通知)。
    fn exchange_result(s: &mut McpSession, msg: Value) -> Value {
        exchange(s, msg)
            .into_iter()
            .find(|r| r.get("id").is_some())
            .expect("应有带 id 的响应")
    }

    fn initialize(s: &mut McpSession, client_name: &str) -> Value {
        exchange_result(
            s,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "clientInfo": { "name": client_name } }
            }),
        )
    }

    /// 走完整 tools/call 信封,返回 result 部分(content/structuredContent/isError)。
    fn call_via(s: &mut McpSession, tool: &str, args: Value) -> Value {
        exchange_result(
            s,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": tool, "arguments": args } }),
        )["result"]
            .clone()
    }

    fn ok_payload(result: &Value) -> Value {
        assert_eq!(result["isError"], false, "不应报错: {result}");
        result["structuredContent"].clone()
    }

    fn err_code(result: &Value) -> String {
        assert_eq!(result["isError"], true, "应报错: {result}");
        result["structuredContent"]["error"]["code"].as_str().unwrap().to_string()
    }

    // ---------------- v1.1.1 既有语义回归(业务层) ----------------

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

    // ---------------- v1.3.0 协议层:握手 / actor / 日志 ----------------

    #[test]
    fn initialize_handshake_derives_actor_and_emits_ready_log() {
        let dir = tmp_dir("handshake");
        let mut s = McpSession::new(&dir, "");
        let lines = exchange(
            &mut s,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "clientInfo": { "name": "codex-smoke" } }
            }),
        );
        assert_eq!(lines.len(), 2, "initialize 响应 + ready 日志通知");
        let result = &lines[0]["result"];
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["serverInfo"]["name"], "taskasion");
        assert!(result["instructions"].as_str().unwrap().contains("task_list"));
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert!(result["capabilities"]["logging"].is_object());
        assert_eq!(lines[1]["method"], "notifications/message");
        assert_eq!(lines[1]["params"]["level"], "info");
        assert!(lines[1]["params"]["data"].as_str().unwrap().contains("agent:mcp:codex-smoke"));
        assert_eq!(s.actor, "agent:mcp:codex-smoke");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn actor_resolution_flag_beats_client_name() {
        let dir = tmp_dir("actor-flag");
        let mut s = McpSession::new(&dir, "agent:codex");
        initialize(&mut s, "zcode");
        assert_eq!(s.actor, "agent:codex", "--actor 显式指定时优先于 clientInfo 派生");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_client_name_rules() {
        assert_eq!(sanitize_client_name("codex-smoke"), "codex-smoke");
        assert_eq!(sanitize_client_name("Codex CLI!"), "Codex-CLI");
        assert_eq!(sanitize_client_name("--zcode--"), "zcode");
        assert_eq!(sanitize_client_name(""), "");
        let long = sanitize_client_name(&"a".repeat(50));
        assert_eq!(long.len(), 32);
    }

    #[test]
    fn unknown_tool_is_protocol_error_32602_but_business_errors_stay_iserror() {
        let dir = tmp_dir("unknown-tool");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        // 未知工具 → 协议错误(规范示例语义)
        let resp = exchange_result(
            &mut s,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "nope" } }),
        );
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("nope"));
        // 已知工具的业务失败 → isError 结果,structuredContent.error 带错误码
        let result = call_via(&mut s, "task_delete", json!({ "task_id": "deadbeef" }));
        assert_eq!(err_code(&result), "not_found");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_error_emits_log_notification_after_init() {
        let dir = tmp_dir("err-log");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        let lines = exchange(
            &mut s,
            json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"task_delete","arguments":{"task_id":"deadbeef"}}}),
        );
        assert_eq!(lines.len(), 2, "错误结果 + error 日志通知");
        assert_eq!(lines[0]["result"]["isError"], true);
        assert_eq!(lines[1]["method"], "notifications/message");
        assert_eq!(lines[1]["params"]["level"], "error");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn notifications_and_bad_json_get_no_reply() {
        let dir = tmp_dir("notify");
        let mut s = McpSession::new(&dir, "");
        assert!(exchange(&mut s, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_empty());
        let mut buf = Vec::new();
        s.handle_line("not-json", &mut buf).unwrap();
        assert!(buf.is_empty());
        // 未知方法仍 -32601
        let resp = exchange_result(&mut s, json!({"jsonrpc":"2.0","id":4,"method":"foo/bar"}));
        assert_eq!(resp["error"]["code"], -32601);
        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------- v1.3.0 工具面:annotations / structuredContent / 新能力 ----------------

    #[test]
    fn annotations_mark_readonly_and_destructive_tools() {
        assert_eq!(tools().len(), 16);
        let readonly: Vec<&str> = tools()
            .iter()
            .filter(|t| t.annotations["readOnlyHint"] == true)
            .map(|t| t.name)
            .collect();
        assert_eq!(readonly, vec!["task_list", "task_plan_today", "goal_list", "capabilities"]);
        let destructive: Vec<&str> = tools()
            .iter()
            .filter(|t| t.annotations["destructiveHint"] == true)
            .map(|t| t.name)
            .collect();
        assert_eq!(destructive, vec!["task_delete", "goal_delete"]);
        for t in tools() {
            assert_eq!(t.annotations["openWorldHint"], false);
        }
    }

    #[test]
    fn structured_content_mirrors_text_and_is_error_false() {
        let dir = tmp_dir("structured");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        let result = call_via(&mut s, "task_add", json!({ "title": "交季度报告" }));
        let payload = ok_payload(&result);
        assert_eq!(payload["title"], "交季度报告");
        let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, payload, "text 与 structuredContent 应是同一个 JSON 值");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_add_goal_id_links_immediately() {
        let dir = tmp_dir("add-goal");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        let gid = ok_payload(&call_via(&mut s, "goal_add", json!({ "title": "健身" })))["id"]
            .as_str()
            .unwrap()
            .to_string();
        let payload = ok_payload(&call_via(&mut s, "task_add", json!({ "title": "跑步 5km", "goal_id": gid })));
        assert_eq!(payload["tags"], json!([format!("goal:{gid}")]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_unlink_task_is_idempotent_and_errors_when_task_missing() {
        let dir = tmp_dir("unlink");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        let gid = ok_payload(&call_via(&mut s, "goal_add", json!({ "title": "学 Rust" })))["id"]
            .as_str()
            .unwrap()
            .to_string();
        let tid = ok_payload(&call_via(&mut s, "task_add", json!({ "title": "读 TRPL" })))["id"]
            .as_str()
            .unwrap()
            .to_string();
        // link(幂等)→ unlink → 再 unlink(无该标签也不报错)
        ok_payload(&call_via(&mut s, "goal_link_task", json!({ "goal_id": gid, "task_id": tid })));
        ok_payload(&call_via(&mut s, "goal_link_task", json!({ "goal_id": gid, "task_id": tid })));
        let payload = ok_payload(&call_via(&mut s, "goal_unlink_task", json!({ "goal_id": gid, "task_id": tid })));
        assert_eq!(payload["tags"], json!([] as [Value; 0]));
        ok_payload(&call_via(&mut s, "goal_unlink_task", json!({ "goal_id": gid, "task_id": tid })));
        // 任务不存在 → not_found
        assert_eq!(
            err_code(&call_via(&mut s, "goal_unlink_task", json!({ "goal_id": gid, "task_id": "deadbeef" }))),
            "not_found"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_list_query_filters_title_and_note() {
        let dir = tmp_dir("query");
        let mut s = McpSession::new(&dir, "");
        initialize(&mut s, "x");
        ok_payload(&call_via(&mut s, "task_add", json!({ "title": "买牛奶", "note": "全脂" })));
        ok_payload(&call_via(&mut s, "task_add", json!({ "title": "写周报" })));
        let mut count = |q: &str| -> usize {
            // task_list 的结果就是裸数组(v1.1.1 起的形状,不包对象)
            ok_payload(&call_via(&mut s, "task_list", json!({ "query": q })))
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!(count("牛奶"), 1, "query 命中标题");
        assert_eq!(count("全脂"), 1, "query 命中备注");
        assert_eq!(count("周报"), 1);
        assert_eq!(count("不存在"), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
