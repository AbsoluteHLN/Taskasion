# 竞品调研摘要(2026-09-14,数据来自 GitHub API 实查)

结论:**"桌面悬浮待办 UI + 本地优先数据 + Agent 可编程接入(MCP/REST)"三者合一的项目目前是空位**,这是 Taskasion 的定位依据。

## A. 悬浮待办小部件(最接近,但都无 Agent 接口)

| 项目 | 星数 | 技术栈 | 可借鉴 |
|---|---|---|---|
| [BUG-gao/floating-todo](https://github.com/BUG-gao/floating-todo) | 12★ 活跃 | Tauri 2 | macOS+Win 半透明置顶;"今天/明天/后天"聚焦;全局快捷键秒记;自然语言设提醒;打卡 streak;纯本地 |
| [XGxin/ApexTodo](https://github.com/XGxin/ApexTodo) | 26★ 活跃 | Electron+React | 数据为单个 `todo.md`;桌面嵌入+鼠标穿透;WebDAV 同步;显示 Codex 用量 |
| 长尾 | 0-3★ ×10 | Py/AHK/WPF | Easy-TodoList、Floatodo、task-widget、MyTime、open-todolist(WPF+SQLite)等 |

赛道 2026 年明显升温,但无一项目过 50★——头部空缺。

## B. 灵动岛/悬浮岛(星数高,2026 年集体转向"Agent 状态栏")

| 项目 | 星数 | 要点 |
|---|---|---|
| DynamicWin | 597★ | C#,通用悬浮组件平台 |
| eIsland / WinIsland / NetSpeed-Dynamic | 313/279/283★ | Electron / Rust / Vue,活跃 |
| [FocuSD](https://github.com/zzliu93-debug/FocuSD) | 210★ | Tauri 2+React 19:待办+笔记+剪贴板+Codex 状态灯,**Codex hooks 一键安装**值得抄;待办存本地 `YYYY-MM-DD.md` |
| [EchoIsland](https://github.com/FunplayAI/EchoIsland) | 69★ | Tauri+Rust,聚合 Codex/Claude Code 会话状态——"Agent 状态→桌面"方向 |

## C. macOS 菜单栏 todo

FocusedTask 147★(2023 停更)等;闭源 One Thing / TodoBar。Apple 生态,参考价值有限。

## D. Agent 接入现状(验证了需求,缺的正是桌面端)

| 项目 | 星数 | 说明 |
|---|---|---|
| abhiz123/todoist-mcp-server | 391★ | Todoist MCP |
| jacepark12/ticktick-mcp | 298★ | 滴答官方 Open API |
| greirson/mcp-todoist | 245★ | 批量操作 |
| jordanburke/microsoft-todo-mcp-server | 106★ | 仍在日更 |
| didatodolist-mcp | 84★ | TickTick 任务+goal |
| flesler/mcp-tasks | 47★ | **本地 md/json/yaml 任务 + 5 个精简 MCP 工具**,token 效率设计,最佳先行参考;无 UI |
| ticktick-mcp-enhanced | 32★ | 中文,滴答官方 API 本地 MCP |
| EricZeng599/nanoSecretary | — | 悬浮球+Ollama,一句话自动提取待办(AI→任务单向) |

**空白**:没有任何项目同时做到 悬浮 UI + MCP 管理 + Agent 变更审计/确认机制。

## 方法论备注

GitHub 数据(星数/推送时间)当日经 API 核实;闭源商业产品(One Thing、滴答清单 Windows 端、Rainmeter 皮肤等)基于公开常识,未逐一验证(本机网络当时无法访问部分搜索源)。
