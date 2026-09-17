//! 本地 REST API(std::net 手写 HTTP/1.1,零第三方依赖,仅监听本机回环)。
//!
//! 路由、状态码、CORS、错误语义与原 Python 版 http.server 实现等价:
//! - 身份取 X-Taskasion-Actor 头(缺省 human),全部变更入审计;
//! - CoreError::NotFound → 404、Invalid → 400、Internal → 500,body 一律 {"error": ...};
//! - 未匹配路由 → 404 {"error":"not_found"};OPTIONS → 204。
//! 与 Python 版唯一差异:请求体只按 UTF-8 解码(去掉了 GBK 兜底)。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::audit::Audit;
use super::models::Task;
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

/// 启动 REST 服务(阻塞当前线程)。绑定失败按 250ms 间隔重试
/// (更新换 exe 后新实例要等旧进程释放端口),重试耗尽则返回错误。
pub fn serve(host: &str, port: u16, data_dir: &Path, bind_retries: usize) -> std::io::Result<()> {
    let core = Arc::new(Core::open(data_dir));
    let listener = bind_with_retry(host, port, bind_retries)?;
    println!(
        "taskasion-core {} listening on http://{host}:{port}  data={}",
        VERSION,
        data_dir.display()
    );
    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let core = core.clone();
            std::thread::spawn(move || handle_conn(stream, &core));
        }
    }
    Ok(())
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
    Ok(Some(Request { method, path, query, actor, body }))
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
        500 => "Internal Server Error",
        _ => "OK",
    };
    let body = payload.map(|v| serde_json::to_vec(v).unwrap_or_default()).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, PATCH, DELETE, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type, X-Taskasion-Actor\r\n\
         Connection: close\r\n\r\n",
        code,
        reason,
        body.len(),
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
fn goal_dict(core: &Core, goal: &Task) -> Value {
    let tasks = core.store.list("all", Some(&format!("goal:{}", goal.id)));
    let done = tasks.iter().filter(|t| t.done).count();
    let mut data = serde_json::to_value(goal).unwrap_or(Value::Null);
    data["progress"] = json!({ "total": tasks.len(), "done": done });
    data
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
        _ => Ok((404, Some(json!({ "error": "not_found" })))),
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
        "/api/goals" => {
            let status = query_get(&req.query, "status").unwrap_or("all");
            let list: Vec<Value> = core.goals.list(status).iter().map(|g| goal_dict(core, g)).collect();
            return Ok((200, Some(Value::Array(list))));
        }
        _ => {}
    }
    Ok((404, Some(json!({ "error": "not_found" }))))
}

fn handle_post(core: &Core, req: &Request, body: Option<Value>) -> Reply {
    if req.path == "/api/tasks" {
        let body = body.ok_or_else(|| CoreError::Invalid("请求体不是合法 JSON".into()))?;
        let task = core.store.add(
            body.get("title").and_then(Value::as_str).unwrap_or(""),
            body.get("due").and_then(Value::as_str),
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
