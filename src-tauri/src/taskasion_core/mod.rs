//! Taskasion 本地核心(Rust 版):todo.md/goals.md 真相源 + REST + MCP + 审计。
//!
//! 由原 Python 版 taskasion_core(标准库实现)移植而来,行为等价:
//! REST API、MCP 工具集、todo.md/goals.md/audit.jsonl 的数据格式完全兼容。
//! 并入桌面壳进程后,不再需要独立的 Python 运行时。

pub mod audit;
pub mod mcp;
pub mod models;
pub mod rest;
pub mod store;

pub use store::{CoreError, GoalStore, TaskStore};

/// 与 Cargo 包版本同源,替代原先 Python 端的重复常量。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
