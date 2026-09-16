# Taskasion

桌面顶层悬浮的 todo / task / goal 小组件,本地优先,内置 **MCP server**,让 Claude Code、Codex 等 AI Agent 可以直接接管任务的增删改查——人看,Agent 管。

```
┌──────────────────────────┐
│  Taskasion (Tauri 2 壳)   │  透明置顶悬浮窗,React/TS 渲染
│  ┌────────────────────┐  │
│  │  todo.md(真相源)   │◄─┼── 人 / Agent / 脚本 任何一方可直接编辑
│  └────────▲─────────────┘  │
│           │ 127.0.0.1 HTTP │
│  ┌────────┴──────────────┐ │
│  │  taskasion-core (Py)  │  ← REST + 审计 + 提醒
│  │  └─ MCP server(stdio) │◄─── Claude Code / Codex / 任意 MCP 客户端
│  └───────────────────────┘  │
└──────────────────────────┘
```

- **壳**:`src-tauri/` — Tauri 2 + React 18 + TypeScript(Vite)。透明、无边框、始终置顶、贴右侧停靠、`Ctrl+Shift+Space` 全局唤起。
- **核心**:Python 常驻进程(`core/`),任务真相源为单个 `todo.md`(Markdown),内嵌本地 REST API 与 **MCP server(stdio)**,所有变更写入审计日志。
- **Agent 接入三通道**:① 直接编辑 `todo.md`(零门槛);② MCP 工具(推荐,5+1 个工具);③ 本地 REST(`X-Taskasion-Actor` 头标识身份,全部入审计)。

## 快速开始(开发)

```bash
# 1. 启动 Core(零依赖即可跑 HTTP API;MCP 需 mcp 包)
python -m taskasion_core serve          # 需先: PYTHONPATH=core
#    或: cd core && python -m taskasion_core serve

# 2. 前端
pnpm install
pnpm dev            # http://localhost:14410

# 3. 桌面壳(首次编译较久,Rust 依赖较多)
pnpm tauri dev

# 测试
python -m unittest discover -s core/tests
```

数据目录:默认 `%USERPROFILE%\.taskasion`,可用 `TASKASION_DATA_DIR` 覆盖;端口默认 `14411`,`TASKASION_PORT` 可改。

详见 [docs/architecture.md](docs/architecture.md)(含 todo.md 数据格式契约与 MCP 工具集定义)与 [docs/research.md](docs/research.md)(竞品调研)。

## 界面引擎(HLN ui-system v2.3)

UI 由作者自己的 [HLN ui-system v2.3](Industrial-Tactical Vector / arknights 主题)驱动,其构建产物以快照形式内置在 `vendor/hln-ui-system-v2.3/`。本机存在引擎仓库时,Vite 自动直连引擎 `dist`(改动即时生效);否则使用快照。引擎更新后可运行 `pnpm run sync:hln` 刷新快照。

## License

MIT,详见 [LICENSE](LICENSE);第三方组件清单与许可证全文见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
