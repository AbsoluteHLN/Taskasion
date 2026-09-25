//! 本地 REST API(std::net 手写 HTTP/1.1,零第三方依赖,仅监听本机回环)。
//!
//! 路由、状态码、CORS、错误语义与原 Python 版 http.server 实现等价:
//! - 身份取 X-Taskasion-Actor 头(缺省 human),全部变更入审计;
//! - CoreError::NotFound → 404、Invalid → 400、Internal → 500,body 一律 {"error": ...};
//! - 未匹配路由 → 404 {"error":"not_found"};OPTIONS → 204。
//! 与 Python 版唯一差异:请求体只按 UTF-8 解码(去掉了 GBK 兜底)。
//!
//! **Integration API v1**:`/api/v1/*` 是给外部集成(Agent / QQ Bridge / 脚本)的稳定契约,
//! `/api/*` 是历史路径。两者共用同一套 handler 与同一批 Core 领域方法 ——
//! v1 只是在入口处剥掉 `/v1` 段,不存在第二份实现,也就不会出现两套语义漂移。
//! 仅 `/api/v1/capabilities`(以及等价的 `/api/capabilities`)是 v1 专属的能力自述。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::audit::Audit;
use super::models::{Task, REMIND_PREFIX};
use super::store::{CoreError, GoalStore, TaskStore};
use super::VERSION;

/// REST 与 MCP 共用的核心装配:两个 store 共享同一个审计器。
pub struct Core {
    pub store: TaskStore,
    pub goals: GoalStore,
    pub audit: Arc<Audit>,
    pub data_dir: PathBuf,
}

impl Core {
    pub fn open(data_dir: &Path) -> Core {
        let audit = Arc::new(Audit::new(&data_dir.join("audit.jsonl")));
        let store = TaskStore::new(data_dir, Some(audit.clone()));
        let goals = GoalStore::new(data_dir, Some(audit.clone()));
        Core { store, goals, audit, data_dir: data_dir.to_path_buf() }
    }
}

/// 未指定 --data-dir 时的数据目录,与 Python 版一致(TASKASION_DATA_DIR → ~/.taskasion)。
pub fn default_data_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("TASKASION_DATA_DIR") {
        if !env_dir.is_empty() {
            return PathBuf::from(env_dir);
        }
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".taskasion")
}

/// 用现成的 Core 起服务:桌面壳要把**同一个** Core 同时交给 REST 与提醒调度器,
/// 避免同进程出现两份内存态(真相源仍是一份,但少一层不必要的分叉)。
pub fn serve_shared(core: Arc<Core>, host: &str, port: u16, bind_retries: usize) -> std::io::Result<()> {
    let listener = bind_with_retry(host, port, bind_retries)?;
    println!(
        "taskasion-core {} listening on http://{host}:{port}  data={}",
        VERSION,
        core.data_dir.display()
    );
    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let core = core.clone();
            std::thread::spawn(move || handle_conn(stream, &core));
        }
    }
    Ok(())
}

/// 启动 REST 服务(阻塞当前线程)。绑定失败按 250ms 间隔重试
/// (更新换 exe 后新实例要等旧进程释放端口),重试耗尽则返回错误。
pub fn serve(host: &str, port: u16, data_dir: &Path, bind_retries: usize) -> std::io::Result<()> {
    serve_shared(Arc::new(Core::open(data_dir)), host, port, bind_retries)
}

fn bind_with_retry(host: &str, port: u16, retries: usize) -> std::io::Result<TcpListener> {
    let mut last = std::io::Error::new(std::io::ErrorKind::AddrInUse, "bind failed");
    for _ in 0..=retries {
        match TcpListener::bind((host, port)) {
            Ok(l) => return Ok(l),
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(last)
}

// ---------------------------------------------------------------- HTTP 基础

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    actor: String,
    body: Vec<u8>,
}

/// 解析一个 HTTP/1.1 请求(请求行 + 头 + 按 Content-Length 的 body)。
fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None); // 连接已关闭
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    if method.is_empty() {
        return Ok(None);
    }

    let mut content_length = 0usize;
    let mut actor = "human".to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            } else if name == "x-taskasion-actor" && !value.is_empty() {
                actor = value.to_string();
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    // Python 版只在 query 上做 unquote_plus,path 保持原样参与路由比较
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target, Vec::new()),
    };
    // Integration API v1:`/api/v1/xxx` 与 `/api/xxx` 是同一批路由,
    // 在入口剥掉 `/v1` 段后交给完全相同的 handler,因此不可能出现两套语义。
    let path = normalize_path(&path);
    Ok(Some(Request { method, path, query, actor, body }))
}

/// `/api/v1/xxx` → `/api/xxx`;其余原样返回。v1 只是历史路径的稳定别名。
fn normalize_path(path: &str) -> String {
    match path.strip_prefix("/api/v1/") {
        Some(rest) => format!("/api/{rest}"),
        None => path.to_string(),
    }
}

/// parse_qs 简化版:%XX 解码、'+' 视作空格、空值丢弃(与 Python 默认一致)。
fn parse_query(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in raw.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let value = percent_decode(v);
        if value.is_empty() {
            continue;
        }
        out.push((percent_decode(k), value));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
                {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_get<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn write_response(stream: &mut TcpStream, code: u16, payload: Option<&Value>) {
    let reason = match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let body = payload.map(|v| serde_json::to_vec(v).unwrap_or_default()).unwrap_or_default();
    let mut head = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n",
        body.len(),
    );
    if code == 405 {
        head.push_str("Allow: GET, POST, PATCH, DELETE, OPTIONS\r\n");
    }
    head.push_str(
        "Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type, X-Taskasion-Actor\r\n\
         Connection: close\r\n\r\n",
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

// ---------------------------------------------------------------- 路由

/// 统一错误映射:NotFound→404、Invalid→400、Internal→500。
fn err_payload(err: CoreError) -> (u16, Value) {
    let code = match &err {
        CoreError::NotFound(_) => 404,
        CoreError::Invalid(_) => 400,
        CoreError::Internal(_) => 500,
    };
    (code, json!({ "error": err.to_string() }))
}

/// goal.to_dict() 附带关联任务进度(tags 含 goal:<id> 的任务统计)。
pub(crate) fn goal_dict(core: &Core, goal: &Task) -> Value {
    let tasks = core.store.list("all", Some(&format!("goal:{}", goal.id)));
    let done = tasks.iter().filter(|t| t.done).count();
    let mut data = serde_json::to_value(goal).unwrap_or(Value::Null);
    data["progress"] = json!({ "total": tasks.len(), "done": done });
    data
}

/// 约定的身份取值。Core **不解析**这些值的含义(不懂 QQ 号/群号/协议),
/// 只把它们原样写进审计,便于事后区分人与 Agent。
const KNOWN_ACTORS: [&str; 9] = [
    "human",
    "agent:codex",
    "agent:deepseek",
    "agent:claude",
    "agent:mcp",
    "bot:qq",
    "bot:astrbot",
    "external",
    "scheduler",
];

/// `/api/capabilities`:让外部集成(Agent、QQ Bridge)自描述式地发现能力,
/// 不必读源码或写死工具清单。纯静态描述 + 运行时端口/数据目录。
/// MCP 的 `capabilities` 工具复用同一份描述,避免两处清单漂移。
pub(crate) fn capabilities(core: &Core) -> Value {
    json!({
        "api": "taskasion-integration-api",
        "api_version": 1,
        "core_version": VERSION,
        "base_paths": ["/api/v1", "/api"],
        "note": "/api/v1/* 与 /api/* 是同一套实现,/api/* 为历史路径保留。",
        "transports": ["rest", "mcp"],
        "rest": {
            "host": "127.0.0.1",
            "port": 14411,
            "actor_header": "X-Taskasion-Actor",
            "default_actor": "human",
        },
        "mcp": {
            "transport": "stdio",
            "entry": "Taskasion.exe mcp [--data-dir DIR] [--actor NAME]",
            "default_actor": "agent:mcp",
            "actor_resolution": "--actor > env TASKASION_MCP_ACTOR > clientInfo.name 派生(agent:mcp:<name>) > agent:mcp",
            "features": ["instructions", "structured_content", "annotations", "log_notifications"],
        },
        "actors": KNOWN_ACTORS,
        "capabilities": {
            "task": ["list", "get", "add", "update", "complete", "reopen", "delete"],
            "goal": ["list", "get", "add", "update", "complete", "reopen", "delete", "link_task", "unlink_task"],
            "plan": ["today"],
            "audit": ["tail"],
            "reminder": {
                "model": "due + remind_time",
                "remind_time_format": "HH:MM (24 小时制)",
                "storage": format!("todo.md 的 tags 中保留 {} 前缀标签", REMIND_PREFIX),
                "scheduler": "桌面壳进程内轮询,不写 Markdown",
                "note": "remind_time 是 Core 的外部字段,已从 tags 中剥离,不会出现在普通标签里。",
            },
        },
        "task_fields": {
            "id": "string",
            "title": "string",
            "done": "bool",
            "due": "YYYY-MM-DD | null",
            "remind_time": "HH:MM | null",
            "priority": "p1 | p2 | p3 | null",
            "tags": "string[]",
            "note": "string | null",
            "source": "string | null",
            "created": "string | null",
            "done_at": "string | null",
        },
        "null_semantics": "update 时 due / remind_time / note / priority 传 null(或空串)表示清空;MCP 旧接口的 \"none\" 同样被接受。请求体中不出现的字段视为不改动。",
        "data_dir": core.data_dir.display().to_string(),
        "source_of_truth": ["todo.md", "goals.md"],
        "docs": "docs/integration-api.md",
    })
}

/// `GET /api/tasks/{id}` 与 `GET /api/goals/{id}` 共用的按 id 查找。
fn find_task(core: &Core, id: &str) -> Reply {
    match core.store.list("all", None).into_iter().find(|t| t.id == id) {
        Some(t) => Ok((200, Some(serde_json::to_value(&t).unwrap_or(Value::Null)))),
        None => Err(CoreError::NotFound(format!("任务不存在: {id}"))),
    }
}

fn find_goal(core: &Core, id: &str) -> Reply {
    match core.goals.list("all").into_iter().find(|g| g.id == id) {
        Some(g) => Ok((200, Some(goal_dict(core, &g)))),
        None => Err(CoreError::NotFound(format!("目标不存在: {id}"))),
    }
}

fn handle_conn(stream: TcpStream, core: &Core) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(clone) = stream.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    let mut writer = stream;
    let req = match read_request(&mut reader) {
        Ok(Some(req)) => req,
        _ => return,
    };
    let body: Option<Value> = if req.body.is_empty() {
        Some(json!({}))
    } else {
        match serde_json::from_slice::<Value>(&req.body) {
            Ok(v) => Some(v),
            Err(_) => None, // JSON 解析失败 → 400(对应 Python ValueError)
        }
    };
    let (code, payload) = dispatch(core, &req, body);
    write_response(&mut writer, code, payload.as_ref());
}

fn dispatch(core: &Core, req: &Request, body: Option<Value>) -> (u16, Option<Value>) {
    if req.method == "OPTIONS" {
        return (204, None);
    }
    let result = match req.method.as_str() {
        "GET" => handle_get(core, req),
        "POST" => handle_post(core, req, body),
        "PATCH" => handle_patch(core, req, body),
        "DELETE" => handle_delete(core, req),
        _ => {
            if req.path == "/api" || req.path.starts_with("/api/") {
                // 已知 API 前缀 + 不支持的方法 → 405(Allow 头由 write_response 附带)
                return (405, None);
            }
            return (404, Some(json!({ "error": "not_found" })));
        }
    };
    match result {
        Ok(reply) => reply,
        Err(err) => {
            let (code, payload) = err_payload(err);
            (code, Some(payload))
        }
    }
}

type Reply = Result<(u16, Option<Value>), CoreError>;

fn handle_get(core: &Core, req: &Request) -> Reply {
    let path = req.path.as_str();
    match path {
        "/api/health" => {
            return Ok((
                200,
                Some(json!({ "ok": true, "version": VERSION, "data_dir": core.store.data_dir.display().to_string() })),
            ));
        }
        "/api/tasks" => {
            let status = query_get(&req.query, "status").unwrap_or("all");
            let tag = query_get(&req.query, "tag");
            let tasks = core.store.list(status, tag);
            let list: Vec<Value> =
                tasks.iter().map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).collect();
            return Ok((200, Some(Value::Array(list))));
        }
        "/api/audit" => {
            let n_raw = query_get(&req.query, "n").unwrap_or("50");
            let n: usize = match n_raw.parse() {
                Ok(n) => n,
                Err(_) => {
                    // 与 Python 一致:int() 失败走通用异常 → 500
                    return Ok((500, Some(json!({ "error": format!("invalid literal for int() with base 10: '{n_raw}'") }))));
                }
            };
            return Ok((200, Some(Value::Array(core.audit.tail(n)))));
        }
        "/api/plan/today" => return Ok((200, Some(core.store.plan_today()))),
        "/api/capabilities" => return Ok((200, Some(capabilities(core)))),
        "/api/goals" => {
            let status = query_get(&req.query, "status").unwrap_or("all");
            let list: Vec<Value> = core.goals.list(status).iter().map(|g| goal_dict(core, g)).collect();
            return Ok((200, Some(Value::Array(list))));
        }
        _ => {}
    }
    let parts: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    if parts.len() == 3 && parts[0] == "api" {
        if parts[1] == "tasks" {
            return find_task(core, parts[2]);
        }
        if parts[1] == "goals" {
            return find_goal(core, parts[2]);
        }
    }
    Ok((404, Some(json!({ "error": "not_found" }))))
}

fn handle_post(core: &Core, req: &Request, body: Option<Value>) -> Reply {
    if req.path == "/api/tasks" {
        let body = body.ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let task = core.store.add(
            body.get("title").and_then(Value::as_str).unwrap_or(""),
            body.get("due").and_then(Value::as_str),
            body.get("remind_time").and_then(Value::as_str),
            body.get("priority").and_then(Value::as_str),
            body.get("tags").and_then(Value::as_array).map(|arr| {
                arr.iter().filter_map(Value::as_str).map(str::to_string).collect()
            }),
            &req.actor,
            body.get("note").and_then(Value::as_str),
        )?;
        return Ok((201, Some(serde_json::to_value(&task).unwrap_or(Value::Null))));
    }
    let parts: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    if parts.len() == 4 && parts[0] == "api" && parts[1] == "tasks" && matches!(parts[3], "complete" | "reopen") {
        let task = core.store.set_done(parts[2], parts[3] == "complete", &req.actor)?;
        return Ok((200, Some(serde_json::to_value(&task).unwrap_or(Value::Null))));
    }
    if parts.len() == 2 && parts[0] == "api" && parts[1] == "goals" {
        let body = body.ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let goal = core.goals.add(body.get("title").and_then(Value::as_str).unwrap_or(""), &req.actor)?;
        return Ok((201, Some(goal_dict(core, &goal))));
    }
    if parts.len() == 4 && parts[0] == "api" && parts[1] == "goals" && matches!(parts[3], "complete" | "reopen") {
        let goal = core.goals.set_done(parts[2], parts[3] == "complete", &req.actor)?;
        return Ok((200, Some(goal_dict(core, &goal))));
    }
    Ok((404, Some(json!({ "error": "not_found" }))))
}

fn handle_patch(core: &Core, req: &Request, body: Option<Value>) -> Reply {
    let parts: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    if parts.len() == 3 && parts[0] == "api" && parts[1] == "tasks" {
        let body = body.ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let fields = body
            .as_object()
            .cloned()
            .ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let task = core.store.update(parts[2], &fields, &req.actor)?;
        return Ok((200, Some(serde_json::to_value(&task).unwrap_or(Value::Null))));
    }
    if parts.len() == 3 && parts[0] == "api" && parts[1] == "goals" {
        let body = body.ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let goal = core.goals.rename(parts[2], body.get("title").and_then(Value::as_str).unwrap_or(""), &req.actor)?;
        return Ok((200, Some(goal_dict(core, &goal))));
    }
    Ok((404, Some(json!({ "error": "not_found" }))))
}

fn handle_delete(core: &Core, req: &Request) -> Reply {
    let parts: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    if parts.len() == 3 && parts[0] == "api" && parts[1] == "tasks" {
        core.store.delete(parts[2], &req.actor)?;
        return Ok((204, None));
    }
    if parts.len() == 3 && parts[0] == "api" && parts[1] == "goals" {
        core.goals.delete(parts[2], &req.actor)?;
        return Ok((204, None));
    }
    Ok((404, Some(json!({ "error": "not_found" }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taskasion-rest-test-{}-{tag}-{}",
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

    /// 走完整的"路径解析 + v1 归一 + 路由分发",只跳过 socket 收发 ——
    /// 因此这里断言的路由行为就是真实请求的行为。
    fn call(core: &Core, method: &str, target: &str, body: Option<Value>) -> (u16, Option<Value>) {
        let (raw_path, query) = match target.split_once('?') {
            Some((p, q)) => (p, parse_query(q)),
            None => (target, Vec::new()),
        };
        let req = Request {
            method: method.to_string(),
            path: normalize_path(raw_path),
            query,
            actor: "human".to_string(),
            body: Vec::new(),
        };
        dispatch(core, &req, body)
    }

    fn body_of(reply: &(u16, Option<Value>)) -> &Value {
        reply.1.as_ref().expect("该路由应有响应体")
    }

    #[test]
    fn v1_and_legacy_paths_are_the_same_endpoints() {
        let dir = tmp_dir("v1-alias");
        let core = Core::open(&dir);

        let (code, created) = call(
            &core,
            "POST",
            "/api/v1/tasks",
            Some(json!({ "title": "写集成文档", "due": "2026-09-24" })),
        );
        assert_eq!(code, 201);
        let id = body_of(&(code, created.clone()))["id"].as_str().unwrap().to_string();

        // 同一份数据,两条路径都要能取到,且内容一致
        let legacy = call(&core, "GET", "/api/tasks?status=all", None);
        let v1 = call(&core, "GET", "/api/v1/tasks?status=all", None);
        assert_eq!(legacy.0, 200);
        assert_eq!(v1.0, 200);
        assert_eq!(body_of(&legacy), body_of(&v1));

        // 单条读取(v1 新增能力)= 列表里的那一条
        let one = call(&core, "GET", &format!("/api/v1/tasks/{id}"), None);
        assert_eq!(one.0, 200);
        assert_eq!(body_of(&one), &body_of(&v1)[0]);
        assert_eq!(call(&core, "GET", "/api/v1/tasks/deadbeef", None).0, 404);

        // 旧路径的完成/回退/删除照常工作
        assert_eq!(call(&core, "POST", &format!("/api/tasks/{id}/complete"), None).0, 200);
        assert_eq!(body_of(&call(&core, "GET", "/api/v1/tasks", None))[0]["done"], json!(true));
        assert_eq!(call(&core, "POST", &format!("/api/v1/tasks/{id}/reopen"), None).0, 200);
        assert_eq!(call(&core, "DELETE", &format!("/api/v1/tasks/{id}"), None).0, 204);
        assert!(body_of(&call(&core, "GET", "/api/tasks?status=all", None)).as_array().unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn null_and_empty_string_both_clear_but_absent_keys_do_not() {
        let dir = tmp_dir("null-semantics");
        let core = Core::open(&dir);
        let (_, created) = call(
            &core,
            "POST",
            "/api/v1/tasks",
            Some(json!({ "title": "有备注", "note": "原文", "due": "2026-09-24", "remind_time": "14:30" })),
        );
        let id = body_of(&(0, created))["id"].as_str().unwrap().to_string();

        // 字段不出现 = 不改动(PATCH 的基本语义)
        let untouched = call(&core, "PATCH", &format!("/api/v1/tasks/{id}"), Some(json!({})));
        assert_eq!(body_of(&untouched)["note"], json!("原文"));

        // 空串在 REST/领域层就是"清空"(与 Python 版一致)
        let blank = call(&core, "PATCH", &format!("/api/v1/tasks/{id}"), Some(json!({ "note": "" })));
        assert_eq!(body_of(&blank)["note"], Value::Null);

        // null 同样清空,而且能一次清掉多个字段
        call(&core, "PATCH", &format!("/api/v1/tasks/{id}"), Some(json!({ "note": "再写一次" })));
        let cleared = call(
            &core,
            "PATCH",
            &format!("/api/v1/tasks/{id}"),
            Some(json!({ "note": null, "due": null, "remind_time": null })),
        );
        assert_eq!(body_of(&cleared)["note"], Value::Null);
        assert_eq!(body_of(&cleared)["due"], Value::Null);
        assert_eq!(body_of(&cleared)["remind_time"], Value::Null);

        // 非法时刻是客户端错误,不是 500
        let (code, err) = call(
            &core,
            "PATCH",
            &format!("/api/v1/tasks/{id}"),
            Some(json!({ "remind_time": "25:99" })),
        );
        assert_eq!(code, 400);
        assert!(body_of(&(code, err))["error"].as_str().unwrap().contains("HH:MM"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remind_time_round_trips_without_leaking_the_reserved_tag() {
        let dir = tmp_dir("remind-leak");
        let core = Core::open(&dir);
        call(
            &core,
            "POST",
            "/api/v1/tasks",
            Some(json!({ "title": "开会", "due": "2026-09-24", "remind_time": "9:5", "tags": ["work"] })),
        );

        let list = call(&core, "GET", "/api/v1/tasks?status=all", None);
        let t = &body_of(&list)[0];
        assert_eq!(t["remind_time"], json!("09:05"), "容忍 9:5 这样的简写并补零");
        assert_eq!(t["tags"], json!(["work"]), "保留标签不得出现在 tags 里");

        // 真相源文件里它就是一个普通标签:老版本读得懂、也不丢数据
        let md = fs::read_to_string(dir.join("todo.md")).unwrap();
        assert!(md.contains("_remind:09:05"), "应以保留标签形式落盘: {md}");

        // 换一个 Core 实例(等价于重启)读取,提醒仍在
        let reopened = Core::open(&dir);
        let again = call(&reopened, "GET", "/api/v1/tasks?status=all", None);
        assert_eq!(body_of(&again)[0]["remind_time"], json!("09:05"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capabilities_describes_the_same_contract_for_rest_and_mcp() {
        let dir = tmp_dir("capabilities");
        let core = Core::open(&dir);
        let caps = call(&core, "GET", "/api/v1/capabilities", None);
        assert_eq!(caps.0, 200);
        let c = body_of(&caps);

        assert_eq!(c["api"], json!("taskasion-integration-api"));
        assert_eq!(c["api_version"], json!(1));
        assert_eq!(c["core_version"], json!(VERSION));
        assert_eq!(c["mcp"]["default_actor"], json!("agent:mcp"));
        assert_eq!(c["rest"]["default_actor"], json!("human"));
        assert_eq!(c["rest"]["host"], json!("127.0.0.1"), "REST 只能监听回环");
        assert_eq!(c["capabilities"]["reminder"]["model"], json!("due + remind_time"));
        assert!(c["task_fields"]["remind_time"].is_string());
        assert!(c["actors"].as_array().unwrap().iter().any(|a| a == "bot:qq"));

        // 旧路径同样可达(兼容那些已经写死了 /api/capabilities 的客户端)
        assert_eq!(body_of(&call(&core, "GET", "/api/capabilities", None)), c);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goals_keep_progress_and_parity_endpoints() {
        let dir = tmp_dir("goals");
        let core = Core::open(&dir);
        let (code, goal) = call(&core, "POST", "/api/v1/goals", Some(json!({ "title": "AI 建筑" })));
        assert_eq!(code, 201);
        let gid = body_of(&(code, goal))["id"].as_str().unwrap().to_string();

        let (_, task) = call(&core, "POST", "/api/v1/tasks", Some(json!({ "title": "打样" })));
        let tid = body_of(&(0, task))["id"].as_str().unwrap().to_string();
        call(&core, "PATCH", &format!("/api/v1/tasks/{tid}"), Some(json!({ "tags": [format!("goal:{gid}")] })));
        call(&core, "POST", &format!("/api/v1/tasks/{tid}/complete"), None);

        let one = call(&core, "GET", &format!("/api/v1/goals/{gid}"), None);
        assert_eq!(one.0, 200);
        assert_eq!(body_of(&one)["progress"], json!({ "total": 1, "done": 1 }));
        // 列表与单条走同一份 goal_dict,进度不会两处不一致
        assert_eq!(body_of(&call(&core, "GET", "/api/v1/goals", None))[0], *body_of(&one));

        assert_eq!(call(&core, "POST", &format!("/api/v1/goals/{gid}/complete"), None).0, 200);
        assert_eq!(call(&core, "POST", &format!("/api/v1/goals/{gid}/reopen"), None).0, 200);
        assert_eq!(call(&core, "DELETE", &format!("/api/v1/goals/{gid}"), None).0, 204);
        assert_eq!(call(&core, "GET", &format!("/api/v1/goals/{gid}"), None).0, 404);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_paths_stay_404_and_health_reports_the_data_dir() {
        let dir = tmp_dir("misc");
        let core = Core::open(&dir);
        assert_eq!(call(&core, "GET", "/api/v1/nope", None).0, 404);
        assert_eq!(call(&core, "GET", "/api/tasks/x/y/z", None).0, 404);
        let health = call(&core, "GET", "/api/v1/health", None);
        assert_eq!(health.0, 200);
        assert_eq!(body_of(&health)["ok"], json!(true));
        assert_eq!(body_of(&health)["version"], json!(VERSION));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_methods_get_405_on_api_but_404_elsewhere() {
        let dir = tmp_dir("405");
        let core = Core::open(&dir);
        // 已知 API 前缀 + 路由表之外的方法 → 405(不再是伪装的 404)
        assert_eq!(call(&core, "PUT", "/api/v1/tasks", None).0, 405);
        assert_eq!(call(&core, "PUT", "/api/tasks", None).0, 405);
        // OPTIONS 仍直接 204(CORS 预检)
        assert_eq!(call(&core, "OPTIONS", "/api/v1/tasks", None).0, 204);
        // API 前缀之外的未知路径,方法再对不上也是 404
        assert_eq!(call(&core, "PUT", "/definitely-not-api", None).0, 404);
        let _ = fs::remove_dir_all(&dir);
    }
}
