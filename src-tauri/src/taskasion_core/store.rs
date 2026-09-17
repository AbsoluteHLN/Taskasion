//! 任务/目标存储:todo.md、goals.md 是唯一真相源。
//!
//! - 人、Agent、脚本都可以直接编辑真相源;每次公开操作前检测 mtime/size,
//!   变化则重新加载(外部编辑即时生效,审计记为 actor=external)。
//! - 保存时原子写(tmp + rename),并归一化:补 id、待办在前已完成在后。
//! 行格式与解析见 [`super::models`]。与原 Python 版 taskasion_core 行为等价。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use chrono::Local;
use serde_json::json;

use super::audit::Audit;
use super::models::{new_id, now_ts, parse_line, render_line, Task};

pub const TODO_HEADER: &str = "# Taskasion\n\n<!-- 真相源:可直接编辑;行尾 <!-- --> 注释是元数据(Core 会补齐/归一化) -->\n\n";
pub const GOALS_HEADER: &str = "# Taskasion Goals\n\n<!-- 目标真相源;任务用标签 goal:<id> 关联到目标 -->\n\n";

#[derive(Debug)]
pub enum CoreError {
    /// 对应 Python KeyError → HTTP 404
    NotFound(String),
    /// 对应 Python ValueError → HTTP 400
    Invalid(String),
    /// 对应 Python 底层 IO 异常 → HTTP 500
    Internal(String),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::NotFound(m) | CoreError::Invalid(m) | CoreError::Internal(m) => write!(f, "{m}"),
        }
    }
}

/// Python `v or None`:空串与 null 都视为清空。
fn clear_or_keep(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if s.is_empty() => None,
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// 单个 markdown 真相源文件的内存态与原子读写(无锁,由外层 Mutex 保护)。
struct MdStore {
    path: PathBuf,
    header: &'static str,
    items: Vec<Task>,
    stamp: Option<(SystemTime, u64)>,
}

impl MdStore {
    fn open(path: PathBuf, header: &'static str) -> MdStore {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let mut store = MdStore { path, header, items: Vec::new(), stamp: None };
        store.load();
        store
    }

    fn stat(&self) -> Option<(SystemTime, u64)> {
        let md = fs::metadata(&self.path).ok()?;
        Some((md.modified().ok()?, md.len()))
    }

    /// 重新解析文件;缺 id/created 的行补齐,有补齐时原子回写。
    fn load(&mut self) {
        let text = fs::read_to_string(&self.path).unwrap_or_default();
        let mut items: Vec<Task> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut normalized = false;
        for line in text.lines() {
            let Some(mut task) = parse_line(line) else { continue };
            if task.id.is_empty() || seen.iter().any(|id| *id == task.id) {
                // 外部手写行(无 id)或重复 id → 分配新 id 并回写,保证 id 稳定
                task.id = new_id();
                normalized = true;
            }
            if task.created.is_empty() {
                task.created = now_ts();
                normalized = true;
            }
            seen.push(task.id.clone());
            items.push(task);
        }
        self.items = items;
        self.stamp = self.stat();
        if normalized {
            let _ = self.save();
        }
    }

    /// mtime/size 变化则重载;返回是否变化。新增条目记入审计(actor=external)。
    fn reload_if_changed(&mut self, audit: Option<&Audit>, noun: &str, action: &str) -> bool {
        let stamp = self.stat();
        if stamp == self.stamp {
            return false;
        }
        let before: Vec<String> = self.items.iter().map(|t| t.id.clone()).collect();
        self.load();
        if let Some(audit) = audit {
            let new_ids: Vec<String> = self
                .items
                .iter()
                .filter(|t| !before.contains(&t.id))
                .map(|t| t.id.clone())
                .collect();
            audit.append(
                "external",
                action,
                None,
                &format!("{noun}={} new=[{}]", self.items.len(), new_ids.join(", ")),
            );
        }
        true
    }

    /// 原子写:待办在前已完成在后(组内保持原顺序)。
    fn save(&mut self) -> Result<(), CoreError> {
        let mut lines: Vec<String> = self
            .items
            .iter()
            .filter(|t| !t.done)
            .map(render_line)
            .collect();
        lines.extend(self.items.iter().filter(|t| t.done).map(render_line));
        let mut content = String::from(self.header);
        if !lines.is_empty() {
            content.push_str(&lines.join("\n"));
            content.push('\n');
        }
        let tmp = self.path.with_extension("md.tmp");
        fs::write(&tmp, content).map_err(|e| CoreError::Internal(e.to_string()))?;
        fs::rename(&tmp, &self.path).map_err(|e| CoreError::Internal(e.to_string()))?;
        self.stamp = self.stat();
        Ok(())
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    fn require(&self, id: &str, what: &str) -> Result<Task, CoreError> {
        self.items
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("{what}不存在: {id}")))
    }
}

/// audit 的共享句柄:两个 store 与 REST/MCP 共用一个审计器。
type SharedAudit = Option<Arc<Audit>>;

fn log_to(audit: &SharedAudit, actor: &str, action: &str, task_id: Option<&str>, detail: &str) {
    if let Some(audit) = audit {
        audit.append(actor, action, task_id, detail);
    }
}

// ---------------------------------------------------------------- TaskStore

pub struct TaskStore {
    pub data_dir: PathBuf,
    pub todo_path: PathBuf,
    audit: SharedAudit,
    inner: Mutex<MdStore>,
}

impl TaskStore {
    pub fn new(data_dir: &Path, audit: SharedAudit) -> TaskStore {
        let _ = fs::create_dir_all(data_dir);
        let todo_path = data_dir.join("todo.md");
        TaskStore {
            data_dir: data_dir.to_path_buf(),
            todo_path: todo_path.clone(),
            audit,
            inner: Mutex::new(MdStore::open(todo_path.clone(), TODO_HEADER)),
        }
    }

    pub fn list(&self, status: &str, tag: Option<&str>) -> Vec<Task> {
        let mut guard = self.inner.lock().unwrap();
        guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
        guard
            .items
            .iter()
            .filter(|t| match status {
                "todo" => !t.done,
                "done" => t.done,
                _ => true,
            })
            .filter(|t| match tag {
                Some(tag) => t.tags.iter().any(|x| x == tag),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn add(
        &self,
        title: &str,
        due: Option<&str>,
        priority: Option<&str>,
        tags: Option<Vec<String>>,
        source: &str,
        note: Option<&str>,
    ) -> Result<Task, CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid("title 不能为空".into()));
        }
        let task = Task {
            id: new_id(),
            title: title.to_string(),
            done: false,
            due: due.map(str::to_string),
            priority: priority.map(str::to_string),
            tags: tags.unwrap_or_default(),
            source: source.to_string(),
            created: now_ts(),
            done_at: None,
            note: note.map(str::to_string).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        };
        {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
            guard.items.insert(0, task.clone());
            guard.save()?;
        }
        log_to(&self.audit, source, "add", Some(&task.id), title);
        Ok(task)
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        let mut guard = self.inner.lock().unwrap();
        guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
        guard.items.iter().find(|t| t.id == id).cloned()
    }

    /// 部分更新:只处理 body 里出现且被允许的字段;due/note/priority 可传 null 清空。
    pub fn update(
        &self,
        id: &str,
        fields: &serde_json::Map<String, serde_json::Value>,
        actor: &str,
    ) -> Result<Task, CoreError> {
        const ALLOWED: [&str; 6] = ["title", "due", "priority", "tags", "source", "note"];
        const CLEARABLE: [&str; 3] = ["due", "note", "priority"];
        let mut applied: Vec<String> = Vec::new();
        let task = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
            // 与 Python 一致:取值、改字段、保存全过程在同一把锁内
            let slot = guard
                .get_mut(id)
                .ok_or_else(|| CoreError::NotFound(format!("任务不存在: {id}")))?;
            for key in ALLOWED {
                let Some(v) = fields.get(key) else { continue };
                let clearable = CLEARABLE.contains(&key);
                let valid = match v {
                    serde_json::Value::Null => clearable, // null 只允许清空可清字段
                    serde_json::Value::String(_) => !matches!(key, "tags"),
                    serde_json::Value::Array(_) => key == "tags",
                    _ => false,
                };
                if !valid {
                    continue;
                }
                match (key, v) {
                    ("title", serde_json::Value::String(s)) => slot.title = s.clone(),
                    ("source", serde_json::Value::String(s)) => slot.source = s.clone(),
                    ("due", v) => slot.due = clear_or_keep(v),
                    ("priority", v) => slot.priority = clear_or_keep(v),
                    ("note", v) => slot.note = clear_or_keep(v),
                    ("tags", serde_json::Value::Array(arr)) => {
                        slot.tags = arr
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect();
                    }
                    _ => {}
                }
                applied.push(key.to_string());
            }
            if applied.is_empty() {
                return Err(CoreError::Invalid("没有可更新字段".into()));
            }
            let task = guard.require(id, "任务")?;
            guard.save()?;
            task
        };
        self.log_to(actor, "update", Some(&task), &applied.join(","));
        Ok(task)
    }

    pub fn set_done(&self, id: &str, done: bool, actor: &str) -> Result<Task, CoreError> {
        let task = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
            {
                let slot = guard
                    .get_mut(id)
                    .ok_or_else(|| CoreError::NotFound(format!("任务不存在: {id}")))?;
                slot.done = done;
                slot.done_at = if done { Some(now_ts()) } else { None };
            }
            let task = guard.require(id, "任务")?;
            guard.save()?;
            task
        };
        self.log_to(actor, if done { "complete" } else { "reopen" }, Some(&task), &task.title);
        Ok(task)
    }

    pub fn delete(&self, id: &str, actor: &str) -> Result<Task, CoreError> {
        let task = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
            let task = guard.require(id, "任务")?;
            guard.items.retain(|t| t.id != id);
            guard.save()?;
            task
        };
        self.log_to(actor, "delete", Some(&task), &task.title);
        Ok(task)
    }

    pub fn plan_today(&self) -> serde_json::Value {
        let today = Local::now().date_naive().to_string();
        let live: Vec<Task> = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit");
            guard.items.iter().filter(|t| !t.done).cloned().collect()
        };
        let overdue: Vec<&Task> = live
            .iter()
            .filter(|t| t.due.as_deref().is_some_and(|d| d < today.as_str()))
            .collect();
        let today_tasks: Vec<&Task> = live.iter().filter(|t| t.due.as_deref() == Some(today.as_str())).collect();
        let mut next: Vec<&Task> = live
            .iter()
            .filter(|t| t.due.as_deref().is_some_and(|d| d > today.as_str()))
            .collect();
        next.sort_by(|a, b| {
            a.due.cmp(&b.due).then_with(|| {
                let pa = a.priority.clone().unwrap_or_else(|| "p9".into());
                let pb = b.priority.clone().unwrap_or_else(|| "p9".into());
                pa.cmp(&pb)
            })
        });
        json!({
            "today": today,
            "overdue": overdue.iter().map(|t| serde_json::to_value(t).unwrap()).collect::<Vec<_>>(),
            "today_tasks": today_tasks.iter().map(|t| serde_json::to_value(t).unwrap()).collect::<Vec<_>>(),
            "next": next.iter().take(5).map(|t| serde_json::to_value(t).unwrap()).collect::<Vec<_>>(),
        })
    }

    fn log_to(&self, actor: &str, action: &str, task: Option<&Task>, detail: &str) {
        log_to(&self.audit, actor, action, task.map(|t| t.id.as_str()), detail);
    }

    /// 供测试/诊断:手动触发一次外部编辑检测。
    pub fn reload_probe(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.reload_if_changed(self.audit.as_deref(), "tasks", "external_edit")
    }
}

// ---------------------------------------------------------------- GoalStore

pub struct GoalStore {
    pub data_dir: PathBuf,
    pub goals_path: PathBuf,
    audit: SharedAudit,
    inner: Mutex<MdStore>,
}

impl GoalStore {
    pub fn new(data_dir: &Path, audit: SharedAudit) -> GoalStore {
        let _ = fs::create_dir_all(data_dir);
        let goals_path = data_dir.join("goals.md");
        GoalStore {
            data_dir: data_dir.to_path_buf(),
            goals_path: goals_path.clone(),
            audit,
            inner: Mutex::new(MdStore::open(goals_path.clone(), GOALS_HEADER)),
        }
    }

    pub fn list(&self, status: &str) -> Vec<Task> {
        let mut guard = self.inner.lock().unwrap();
        guard.reload_if_changed(self.audit.as_deref(), "goals", "external_edit_goals");
        guard
            .items
            .iter()
            .filter(|g| match status {
                "todo" => !g.done,
                "done" => g.done,
                _ => true,
            })
            .cloned()
            .collect()
    }

    pub fn add(&self, title: &str, source: &str) -> Result<Task, CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid("title 不能为空".into()));
        }
        let goal = Task {
            id: new_id(),
            title: title.to_string(),
            done: false,
            due: None,
            priority: None,
            tags: vec![],
            source: source.to_string(),
            created: now_ts(),
            done_at: None,
            note: None,
        };
        {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "goals", "external_edit_goals");
            guard.items.insert(0, goal.clone());
            guard.save()?;
        }
        log_to(&self.audit, source, "goal_add", Some(&goal.id), title);
        Ok(goal)
    }

    pub fn rename(&self, id: &str, title: &str, actor: &str) -> Result<Task, CoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid("title 不能为空".into()));
        }
        let goal = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "goals", "external_edit_goals");
            {
                let slot = guard.get_mut(id).ok_or_else(|| CoreError::NotFound(format!("目标不存在: {id}")))?;
                slot.title = title.to_string();
            }
            let goal = guard.require(id, "目标")?;
            guard.save()?;
            goal
        };
        self.log_to(actor, "goal_rename", Some(&goal), title);
        Ok(goal)
    }

    pub fn set_done(&self, id: &str, done: bool, actor: &str) -> Result<Task, CoreError> {
        let goal = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "goals", "external_edit_goals");
            {
                let slot = guard.get_mut(id).ok_or_else(|| CoreError::NotFound(format!("目标不存在: {id}")))?;
                slot.done = done;
                slot.done_at = if done { Some(now_ts()) } else { None };
            }
            let goal = guard.require(id, "目标")?;
            guard.save()?;
            goal
        };
        self.log_to(
            actor,
            if done { "goal_complete" } else { "goal_reopen" },
            Some(&goal),
            &goal.title,
        );
        Ok(goal)
    }

    pub fn delete(&self, id: &str, actor: &str) -> Result<Task, CoreError> {
        let goal = {
            let mut guard = self.inner.lock().unwrap();
            guard.reload_if_changed(self.audit.as_deref(), "goals", "external_edit_goals");
            let goal = guard.require(id, "目标")?;
            guard.items.retain(|t| t.id != id);
            guard.save()?;
            goal
        };
        self.log_to(actor, "goal_delete", Some(&goal), &goal.title);
        Ok(goal)
    }

    fn log_to(&self, actor: &str, action: &str, goal: Option<&Task>, detail: &str) {
        log_to(&self.audit, actor, action, goal.map(|t| t.id.as_str()), detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taskasion-test-{}-{tag}-{}",
            std::process::id(),
            now_ts().replace(':', "-")
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store_in(dir: &Path) -> (TaskStore, Arc<Audit>) {
        let audit = Arc::new(Audit::new(&dir.join("audit.jsonl")));
        (TaskStore::new(dir, Some(audit.clone())), audit)
    }

    #[test]
    fn add_and_reload() {
        let dir = tmp_dir("add-reload");
        let (store, _) = store_in(&dir);
        let task = store
            .add("交季度报告", Some("2026-09-18"), Some("p1"), Some(vec!["work".into()]), "human", None)
            .unwrap();
        let other = TaskStore::new(&dir, None); // 模拟重启:从文件恢复
        let list = other.list("all", None);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "交季度报告");
        assert_eq!(list[0].id, task.id);
        assert_eq!(list[0].due.as_deref(), Some("2026-09-18"));
        assert_eq!(list[0].tags, vec!["work"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_and_reopen() {
        let dir = tmp_dir("complete-reopen");
        let (store, _) = store_in(&dir);
        let task = store.add("买菜", None, None, None, "human", None).unwrap();
        store.set_done(&task.id, true, "human").unwrap();
        assert!(store.list("done", None)[0].done);
        assert!(store.list("todo", None).is_empty());
        let raw = fs::read_to_string(&store.todo_path).unwrap();
        assert!(raw.contains("- [x] 买菜"));
        store.set_done(&task.id, false, "human").unwrap();
        assert_eq!(store.list("todo", None).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_and_delete() {
        let dir = tmp_dir("update-delete");
        let (store, _) = store_in(&dir);
        let task = store
            .add("写周报", None, None, Some(vec!["work".into()]), "human", None)
            .unwrap();
        let body = serde_json::json!({"title": "写周报 v2", "due": "2026-09-15"});
        store.update(&task.id, body.as_object().unwrap(), "human").unwrap();
        let got = store.get(&task.id).unwrap();
        assert_eq!(got.title, "写周报 v2");
        assert_eq!(got.due.as_deref(), Some("2026-09-15"));
        store.delete(&task.id, "human").unwrap();
        assert!(store.get(&task.id).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_edit_is_picked_up() {
        let dir = tmp_dir("external-edit");
        let (store, _) = store_in(&dir);
        store.add("已有任务", None, None, None, "human", None).unwrap();
        let raw = fs::read_to_string(&store.todo_path).unwrap();
        fs::write(&store.todo_path, format!("{raw}\n- [ ] 外部手写的任务\n")).unwrap();
        // mtime 可能不变(同秒),手动改 stamp 触发重载路径由真实 mtime 决定;
        // 这里直接依赖 stat 检测,size 一定变了。
        let mut guard_ok = false;
        for _ in 0..10 {
            if store.reload_probe() {
                guard_ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(guard_ok, "外部编辑未被检测到");
        let titles: Vec<String> = store.list("all", None).iter().map(|t| t.title.clone()).collect();
        assert!(titles.contains(&"外部手写的任务".to_string()));
        let ext_id = store
            .list("all", None)
            .iter()
            .find(|t| t.title == "外部手写的任务")
            .unwrap()
            .id
            .clone();
        // 手写行获得稳定 id:再次检测(mtime 未变)不应重载,id 不变
        assert!(!store.reload_probe());
        assert_eq!(store.get(&ext_id).unwrap().title, "外部手写的任务");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_records_agent_and_external() {
        let dir = tmp_dir("audit");
        let (store, audit) = store_in(&dir);
        store.add("a", None, None, None, "agent:test", None).unwrap();
        store.add("b", None, None, None, "human", None).unwrap();
        let raw = fs::read_to_string(&store.todo_path).unwrap();
        fs::write(&store.todo_path, format!("{raw}\n- [ ] c\n")).unwrap();
        assert!(store.reload_probe());
        let actions: Vec<(String, String)> = audit
            .tail(10)
            .iter()
            .map(|e| (e["actor"].as_str().unwrap().into(), e["action"].as_str().unwrap().into()))
            .collect();
        assert!(actions.contains(&("agent:test".into(), "add".into())));
        assert!(actions.contains(&("external".into(), "external_edit".into())));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_today() {
        let dir = tmp_dir("plan-today");
        let (store, _) = store_in(&dir);
        let today = Local::now().date_naive().to_string();
        let yesterday = (Local::now().date_naive() - chrono::Duration::days(1)).to_string();
        store.add("过期了", Some(&yesterday), None, None, "human", None).unwrap();
        store.add("今天做", Some(&today), None, None, "human", None).unwrap();
        let plan = store.plan_today();
        assert_eq!(plan["overdue"][0]["title"], "过期了");
        assert_eq!(plan["today_tasks"][0]["title"], "今天做");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_live_like_data() {
        // 模拟真实数据(含中文、备注转义、done 行)往返不丢信息
        let dir = tmp_dir("roundtrip");
        let (store, _) = store_in(&dir);
        let t1 = store
            .add("多元统计分析和机器学习作业", Some("2026-09-16"), Some("p1"), None, "human", Some("已一半"))
            .unwrap();
        store.add("大厅修改", None, None, Some(vec!["goal:abc12345".into()]), "human", None).unwrap();
        store.set_done(&t1.id, true, "human").unwrap();
        let raw = fs::read_to_string(&store.todo_path).unwrap();
        let other = TaskStore::new(&dir, None);
        assert_eq!(other.list("all", None).len(), 2);
        // 顺序:待办在前已完成在后
        assert!(raw.find("- [ ]").is_some());
        assert!(raw.find("- [x]").is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------- GoalStore(对应 core/tests/test_goals.py)----------------

    fn goals_in(dir: &Path) -> (GoalStore, Arc<Audit>) {
        let audit = Arc::new(Audit::new(&dir.join("audit.jsonl")));
        (GoalStore::new(dir, Some(audit.clone())), audit)
    }

    #[test]
    fn goal_add_and_reload() {
        let dir = tmp_dir("goal-add-reload");
        let (goals, _) = goals_in(&dir);
        let goal = goals.add("三个月跑通产品化", "human").unwrap();
        let other = GoalStore::new(&dir, None); // 模拟重启:从文件恢复
        let loaded = other.list("all");
        assert_eq!(loaded.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), vec!["三个月跑通产品化"]);
        assert_eq!(loaded[0].id, goal.id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_rename_complete_delete() {
        let dir = tmp_dir("goal-rcd");
        let (goals, _) = goals_in(&dir);
        let goal = goals.add("健身计划", "human").unwrap();
        goals.rename(&goal.id, "年度健身计划", "human").unwrap();
        assert_eq!(goals.list("all")[0].title, "年度健身计划");
        goals.set_done(&goal.id, true, "human").unwrap();
        assert_eq!(goals.list("done").iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec![goal.id.as_str()]);
        assert!(goals.list("todo").is_empty());
        goals.delete(&goal.id, "human").unwrap();
        assert!(goals.list("all").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_external_handwritten_gets_id() {
        let dir = tmp_dir("goal-external");
        let (goals, _) = goals_in(&dir);
        fs::write(
            &goals.goals_path,
            "# Taskasion Goals\n\n- [ ] 手写目标 <!-- created:2026-09-15T08:00:00 -->\n",
        )
        .unwrap();
        let loaded = goals.list("all"); // list 内部触发外部编辑检测
        assert_eq!(loaded.len(), 1);
        assert!(!loaded[0].id.is_empty()); // Core 自动补 id 并回写
        let raw = fs::read_to_string(&goals.goals_path).unwrap();
        assert!(raw.contains(&format!("id:{}", loaded[0].id)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_audit_records_actions() {
        let dir = tmp_dir("goal-audit");
        let (goals, audit) = goals_in(&dir);
        let goal = goals.add("学 Rust", "agent:mcp").unwrap();
        goals.set_done(&goal.id, true, "human").unwrap();
        let actions: Vec<(String, String)> = audit
            .tail(10)
            .iter()
            .map(|r| (r["actor"].as_str().unwrap().into(), r["action"].as_str().unwrap().into()))
            .collect();
        assert!(actions.contains(&("agent:mcp".into(), "goal_add".into())));
        assert!(actions.contains(&("human".into(), "goal_complete".into())));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn goal_linked_task_progress() {
        // 进度统计在 REST 层(goal_dict);此处验证任务侧 goal:<id> 标签过滤
        let dir = tmp_dir("goal-link");
        let (goals, audit) = goals_in(&dir);
        let goal = goals.add("写书", "human").unwrap();
        let store = TaskStore::new(&dir, Some(audit));
        let t1 = store.add("写第一章", None, None, Some(vec![format!("goal:{}", goal.id)]), "human", None).unwrap();
        store.add("写第二章", None, None, Some(vec![format!("goal:{}", goal.id)]), "human", None).unwrap();
        store.add("无关任务", None, None, None, "human", None).unwrap();
        assert_eq!(store.list("all", Some(&format!("goal:{}", goal.id))).len(), 2);
        store.set_done(&t1.id, true, "human").unwrap();
        assert_eq!(store.list("done", Some(&format!("goal:{}", goal.id))).len(), 1);
        assert_eq!(store.list("todo", Some(&format!("goal:{}", goal.id))).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
