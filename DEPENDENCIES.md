# 依赖指向

本项目所有依赖的唯一来源是 `E:\dependency-cache`；禁止联网重新安装依赖。

全局规则与机器级指向表的唯一权威是 [`E:\dependency-cache\DEPENDENCY-BOUNDARY.md`](E:\dependency-cache\DEPENDENCY-BOUNDARY.md)，本文只记录本项目特有事项。

## 本项目说明

- 前端 pnpm（`pnpm-workspace.yaml`），桌面壳 Tauri/Rust（`src-tauri/`）。
- pnpm 解析走 `~/.npmrc` 的 `store-dir`（→ `E:\dependency-cache\pnpm-store`）；Rust 经 `CARGO_HOME=E:\dependency-cache\cargo`。
- 端口规划：Vite devUrl 14410，Python core 14411。

## 校验

```sh
pnpm config get store-dir   # 应输出 E:\dependency-cache\pnpm-store
```
