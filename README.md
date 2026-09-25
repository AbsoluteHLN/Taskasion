# Taskasion

> 只是想给自己写一个便捷、舒适的 todo 清单。

Taskasion 是一个置顶悬浮的 todo / goal 小组件，**本地优先**：所有任务就是一份 `todo.md`。人可以直接编辑，AI Agent 通过内置的 MCP server 同样可以直接接管增删改查——**人看，Agent 管**，每一次变更都记入审计日志。

## 特性

- **悬浮即用**：透明无边框、始终置顶、可折叠为迷你条、`Ctrl+Shift+Space` 全局唤起、系统托盘。
- **单文件即全部**：核心已并入桌面壳，绿色便携版只有一个 `Taskasion.exe`（约 4 MB），免安装、无注册表，删除文件夹即完全卸载，数据随目录拷走。
- **todo.md 是唯一真相源**：单文件 Markdown，零门槛；任何编辑器、脚本、Agent 都能直接改，外部修改即时生效。
- **AI Agent 原生**：内置 MCP server（stdio，15 个工具），Claude Code / Codex 接上即管（`Taskasion.exe mcp --actor agent:codex`，无需安装任何依赖）；也可走本地 REST（Integration API v1，`/api/v1` 与历史 `/api` 同一套实现），`X-Taskasion-Actor` 头标识"谁在操作"。
- **目标（Goal）**：目标下挂任务，自动聚合进度。
- **轻量提醒**：任务可设「日期 + 时刻」，到点响一次柔和提示音并高亮该行；没有重复、提前提醒、日历或设置页。提示音内嵌在 exe 里，无需管理任何音频文件。
- **审计日志**：谁、何时、改了什么，全部追加记录在 `audit.jsonl`。
- **自动更新**：托盘菜单"检查更新"或头部更新按钮，从 GitHub Releases 拉取 portable zip 原子换 exe 重启。

## 架构

```text
            悬浮窗 UI（Tauri 2 + React 18 + TypeScript）
                        │  fetch REST（壳只负责渲染，不保存状态）
                        ▼
            taskasion-core（Rust，与壳同进程线程）
            · Integration API v1  127.0.0.1:14411（仅本机，不联网）
            · MCP server（stdio）→ Claude Code / Codex 等
            · 提醒调度（due + remind_time，内嵌提示音）
            · 审计日志   audit.jsonl（追加式）
                        │  热加载 + 原子写（mtime 检测，外部编辑即时生效）
                        ▼
            todo.md —— 唯一真相源（单文件 Markdown）
```

## 快速开始（开发）

```bash
pnpm install
pnpm build          # 产出 dist/
pnpm tauri build    # 首次编译较久；产物 exe + data\ 同目录运行

# 测试（51 个单测，覆盖行解析/存储/目标/审计/Integration API/提醒）
cd src-tauri && cargo test
```

- 数据目录：桌面端固定为 exe 同级 `data\`；独立运行 `Taskasion.exe mcp` / `Taskasion.exe serve` 时默认 `%USERPROFILE%\.taskasion`，可用 `TASKASION_DATA_DIR` 覆盖。
- 端口默认 `14411`（仅本机回环）。
- Agent 接入 MCP：`command` 填 `Taskasion.exe 的完整路径`，`args` 填 `["mcp", "--actor", "agent:codex"]`，`env` 里 `TASKASION_DATA_DIR` 与桌面端数据目录保持一致。

更多设计细节见 [docs/architecture.md](docs/architecture.md)（todo.md 数据格式契约、路由表、MCP 工具集）与 [docs/integration-api.md](docs/integration-api.md)（REST/MCP 统一契约、null 语义、actor 约定、给 Bot Bridge 预留的接口），以及 [docs/research.md](docs/research.md)（立项前的竞品调研）。

## 界面引擎

UI 基于作者自研的 HLN ui-system v2.3 引擎，其构建产物以快照形式内置在 `vendor/hln-ui-system-v2.3/`。本机存在引擎仓库时，Vite 自动直连引擎 `dist`（改动即时生效）；否则使用快照，克隆即可构建。引擎更新后可运行 `pnpm run sync:hln` 刷新快照。

## License

MIT，详见 [LICENSE](LICENSE)；第三方组件清单与许可证全文见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
