# Taskasion

> 最初只是想给自己写一个便捷、舒适的 todo 清单，后来它长成了 AI 也能一起管的桌面悬浮组件。

Taskasion 是一个置顶悬浮的 todo / goal 小组件，**本地优先**：所有任务就是一份 `todo.md`。人可以直接编辑，AI Agent 通过内置的 MCP server 同样可以直接接管增删改查——**人看，Agent 管**，每一次变更都记入审计日志。

## 特性

- **悬浮即用**：透明无边框、始终置顶、可折叠为迷你条、`Ctrl+Shift+Space` 全局唤起、系统托盘。
- **todo.md 是唯一真相源**：单文件 Markdown，零门槛；任何编辑器、脚本、Agent 都能直接改，外部修改即时生效。
- **AI Agent 原生**：内置 MCP server（stdio，10 个工具），Claude Code / Codex 接上即管（需可选依赖 `pip install "mcp>=1.2"`）；也可走本地 REST，`X-Taskasion-Actor` 头标识"谁在操作"。
- **目标（Goal）**：目标下挂任务，自动聚合进度。
- **审计日志**：谁、何时、改了什么，全部追加记录在 `audit.jsonl`。
- **零依赖核心**：核心只用 Python 标准库；可选安装 `mcp` 包启用 MCP。

## 架构

```text
            悬浮窗 UI（Tauri 2 + React 18 + TypeScript）
                        │  fetch REST（壳只负责渲染，不保存状态）
                        ▼
            taskasion-core（Python 标准库，常驻进程）
            · REST API   127.0.0.1:14411（仅本机，不联网）
            · MCP server（stdio）→ Claude Code / Codex 等
            · 审计日志   audit.jsonl（追加式）
                        │  热加载 + 原子写（mtime 检测，外部编辑即时生效）
                        ▼
            todo.md —— 唯一真相源（单文件 Markdown）
```

## 快速开始（开发）

```bash
# 1. 启动 Core（零依赖即可跑 HTTP API；MCP 需 mcp 包）
cd core && python -m taskasion_core serve

# 2. 前端
pnpm install
pnpm dev            # http://localhost:14410

# 3. 桌面壳（首次编译较久，Rust 依赖较多）
pnpm tauri dev

# 测试
cd core && python -m unittest discover -s tests
```

数据目录：默认 `%USERPROFILE%\.taskasion`，可用 `TASKASION_DATA_DIR` 覆盖；端口默认 `14411`，`TASKASION_PORT` 可改。

更多设计细节见 [docs/architecture.md](docs/architecture.md)（todo.md 数据格式契约、REST API、MCP 工具集）与 [docs/research.md](docs/research.md)（立项前的竞品调研）。

## 界面引擎

UI 基于作者自研的 HLN ui-system v2.3 引擎，其构建产物以快照形式内置在 `vendor/hln-ui-system-v2.3/`。本机存在引擎仓库时，Vite 自动直连引擎 `dist`（改动即时生效）；否则使用快照，克隆即可构建。引擎更新后可运行 `pnpm run sync:hln` 刷新快照。

## License

MIT，详见 [LICENSE](LICENSE)；第三方组件清单与许可证全文见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
