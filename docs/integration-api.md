# Taskasion Integration API v1

给 Agent、脚本、Bot 的统一入口。**只有一套业务实现**:REST 与 MCP 都直接调用同一个
Core 领域层(`TaskStore` / `GoalStore`),不存在"REST 一套、MCP 一套"。真相源永远是
`todo.md` / `goals.md`,本 API 不引入任何新状态。

```
人 / Agent / Bot
   │  REST(127.0.0.1:14411)        MCP(stdio)
   └──────────┬──────────────────────────┘
        Integration API v1  ──►  Core 领域层  ──►  todo.md / goals.md
```

## 1. 两个传输

### REST(仅回环)

- `http://127.0.0.1:14411`。**只绑定 `127.0.0.1`**,不监听 `0.0.0.0`,不对外网/LAN 暴露。
- 身份:`X-Taskasion-Actor` 头,缺省 `human`。
- JSON + CORS(便于本地页面调试);错误体 `{"error": "..."}`,状态码 `400`(参数非法)/`404`(不存在)/`500`。

### MCP(stdio)

```bash
Taskasion.exe mcp [--data-dir DIR] [--actor NAME]
```

- 换行分隔 JSON-RPC 2.0,`initialize` / `ping` / `tools/list` / `tools/call`。
- `initialize` 返回 `instructions` 使用说明,并声明 `capabilities`(`tools` + `logging`);
  运行日志经 `notifications/message` 同步给客户端(stderr 保留)。
- `tools/call` 结果 **text + structuredContent 双写**(两者是同一个 JSON 值);
  业务失败 `isError:true` 且 `structuredContent.error` 带错误码(`not_found`/`invalid`/`internal`);
  未知工具 → 协议错误 `-32602`(对齐规范示例);工具定义带规范 `annotations`
  (只读/危险/幂等提示,客户端可据此分级放行)。
- 缺省 actor `agent:mcp`;`--actor agent:codex` 显式指定。**解析顺序**:
  `--actor` > env `TASKASION_MCP_ACTOR` > `clientInfo.name` 派生(如 `agent:mcp:codex`)>
  `agent:mcp` —— 不传也能在审计里区分不同客户端。
- Agent 配置:`command = <Taskasion.exe 完整路径>`,`args = ["mcp", "--actor", "agent:codex"]`。

### 零配置自助接入(推荐给 Agent)

不写任何客户端配置也能接:用户只说"某目录下有 Taskasion",Agent 在该目录运行
`Taskasion.exe onboarding` —— 输出 exe/数据目录/widget 状态与两种接入方式的现成参数;
`onboarding --write` 会把同等内容写成安装目录的 `AGENTS.md`(主流 Agent 客户端自动读取),
之后"告知目录 = 完成接入"。两种接入方式都通过 `capabilities` 自述,不依赖本文档。

## 2. 路径

`/api/v1/*` 与 `/api/*` 是**同一批路由**(入口剥掉 `/v1` 段后交给同一 handler),
因此永远不可能出现两套语义。`/api/*` 为历史客户端保留,新代码请用 `/api/v1/*`。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/health` | 存活 + 版本 + 数据目录 |
| GET | `/api/v1/capabilities` | 能力自述(见 §6) |
| GET | `/api/v1/tasks?status=all\|todo\|done&tag=x` | 任务列表 |
| GET | `/api/v1/tasks/{id}` | 单个任务 |
| POST | `/api/v1/tasks` | 新建:`{title, due?, remind_time?, priority?, tags?, note?}` → `201` |
| PATCH | `/api/v1/tasks/{id}` | 部分更新(见 §4) |
| POST | `/api/v1/tasks/{id}/complete` · `/reopen` | 勾选 / 回退 |
| DELETE | `/api/v1/tasks/{id}` | 删除 → `204` |
| GET | `/api/v1/goals?status=all\|todo\|done` | 目标列表(含实时 `progress`) |
| GET | `/api/v1/goals/{id}` | 单个目标 |
| POST | `/api/v1/goals` | 新建 `{title}` → `201` |
| PATCH | `/api/v1/goals/{id}` | 重命名 `{title}` |
| POST | `/api/v1/goals/{id}/complete` · `/reopen` | 达成 / 重开 |
| DELETE | `/api/v1/goals/{id}` | 删除 → `204` |
| GET | `/api/v1/plan/today` | 今日规划 `{today, overdue, today_tasks, next}` |
| GET | `/api/v1/audit?n=50` | 审计尾部 |

任务与目标的关联:给任务打标签 `goal:<goalId>`(REST `PATCH tags`,MCP `goal_link_task`)。

```bash
curl -s http://127.0.0.1:14411/api/v1/tasks -X POST \
  -H "X-Taskasion-Actor: agent:codex" -H "Content-Type: application/json" \
  -d '{"title":"交季度报告","due":"2026-09-24","remind_time":"14:30","priority":"p1","tags":["work"],"note":"附上模板"}'
```

## 3. Task 字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | 8 位 hex;缺失时 Core 首次加载自动补齐 |
| `title` | string | 标题,不可空 |
| `done` | bool | 是否完成 |
| `due` | `YYYY-MM-DD` \| null | 日期 |
| `remind_time` | `HH:MM` \| null | 24 小时制提醒时刻 |
| `priority` | `p1`\|`p2`\|`p3` \| null | 优先级 |
| `tags` | string[] | 普通标签,**不含**内部保留标签 |
| `note` | string \| null | 备注 |
| `source` | string \| null | 创建者身份 |
| `created` / `done_at` | ISO 时间 \| null | 创建 / 完成时间 |

Goal 与 Task 同构(标题即目标名),额外带 `progress: {total, done}`(按 `goal:<id>` 标签实时汇总)。

### remind_time(提醒)

- 只支持"某一天 + 某个时刻提醒一次",**没有**重复、提前 N 分钟、多级提醒、日历、设置页。
- `due` 语义保持不变(仍是纯日期),因此 `plan_today` 与 Python 版行为一致。
- 内部持久化:写进 `todo.md` 行尾元数据的 `tags:` 里,用保留标签 `_remind:HH:MM`。
  例如 `tags:work,_remind:14:30`。
- 该保留标签在 Core 解析时被剥离成外部字段 `remind_time`,**永不出现在** `tags` 里,
  也不会通过 REST / MCP / UI 泄露给调用方;渲染时再写回,`todo.md` 仍可被人直接读写。
- 旧版 Python/Rust 会把它当成一个普通标签原样保留,所以双向兼容、不会丢数据。
- 非法格式(`25:99`、`14:3:0`)在 REST/领域层返回 `400`;解析历史文件时非法值被直接丢弃不外泄。

## 4. null 语义

**请求体里不出现的字段 = 不改动**;出现且为清空值 = 清空。

| 值 | REST / 领域层 | MCP |
|---|---|---|
| 字段缺失 | 不改动 | 不改动 |
| `null` | 清空 `due`/`remind_time`/`note`/`priority` | 清空(同左) |
| `""` | 清空(与 Python 版一致) | **视为"未传"**,不改动 |
| `"none"` | 无特殊含义 | 清空(旧约定,保留兼容) |

MCP 的 `""` 例外是刻意的:历史 Agent 大量发送空串表示"没填",不能把它们理解成删除。
只有 `due` / `remind_time` / `note` / `priority` 可清空;`title` / `tags` / `source` 不接受 `null`。

## 5. actor 约定

Core **不解析** actor 的含义(不懂 QQ 号、群号、协议),只原样写进 `audit.jsonl`,便于事后区分:

```
human · agent:codex · agent:deepseek · agent:claude · agent:mcp · bot:qq · bot:astrbot · external · scheduler
```

- REST:头 `X-Taskasion-Actor`,缺省 `human`。
- MCP:解析顺序 `--actor` > env `TASKASION_MCP_ACTOR` > `clientInfo.name` 派生(`agent:mcp:<name>`)> 缺省 `agent:mcp`。
- 未列入的取值不会被拒绝(自由文本),但建议沿用上表。
- 外部直接编辑 `todo.md` 时,Core 热加载后记为 `actor=external`。

## 6. 能力自述

`GET /api/v1/capabilities`(MCP 的 `capabilities` 工具返回同一份结构)给出:REST/MCP 入口与
端口、actor 清单、Task 字段与 null 语义、提醒模型、真相源文件、本文档路径。集成方应读它而不是
写死清单——两端共用一份描述,不会漂移。

## 7. MCP 工具(16 个)

| 工具 | 要点 |
|---|---|
| `task_add` | title 必填;due/remind_time/priority/tags/note 可选;`goal_id` 传目标 id 则创建即关联 |
| `task_list` | status 默认 `todo`;tag 过滤;`query` 按标题/备注子串过滤(大小写不敏感) |
| `task_update` | 未传字段不动;清空传 `null` 或旧写法 `"none"` |
| `task_complete` / `task_reopen` / `task_delete` | 按 `task_id` |
| `task_plan_today` | `{today, overdue, today_tasks, next}` |
| `goal_add` / `goal_list` | title 必填 / status 默认 `todo`,含 progress |
| `goal_update` | 重命名 `{goal_id, title}` |
| `goal_complete` / `goal_reopen` / `goal_delete` | 达成 / 重开 / 删除 |
| `goal_link_task` | 给任务追加 `goal:<id>` 标签(幂等) |
| `goal_unlink_task` | 移除任务的 `goal:<id>` 标签(幂等) |
| `capabilities` | 同 `/api/v1/capabilities` |

annotations 约定:`task_list` / `task_plan_today` / `goal_list` / `capabilities` 标只读;
`task_delete` / `goal_delete` 标危险;完成/回退/关联类标幂等;全部 `openWorldHint:false`。
前 10 个工具的签名与旧 Python FastMCP 版逐字一致,旧调用无需改动。

## 8. 给 Bot Bridge 预留的接口(尚未实现)

**Core 不包含任何 QQ / OneBot / AstrBot 逻辑**:不知道群号、QQ 号、消息事件、群协议。
Bot 侧只准通过本 Integration API 读写,愈合(协议差异、权限、限流、命令解析)全部留在 Bridge 里:

```
Taskasion  ──  Integration API v1  ──  Taskasion QQ Bridge  ──  OneBot / AstrBot
```

Bridge 侧的命令解析示例(全部只是对本文档接口的调用):

| 聊天命令 | 调用的接口 |
|---|---|
| `/todo` | `GET /api/v1/tasks?status=todo` |
| `/addtodo 交报告 2026-09-24` | `POST /api/v1/tasks` |
| `/done a1b2c3d4` | `POST /api/v1/tasks/a1b2c3d4/complete` |
| `/remind a1b2c3d4 14:30` | `PATCH /api/v1/tasks/a1b2c3d4 {"remind_time":"14:30"}` |

Bridge 进程用 `X-Taskasion-Actor: bot:qq` 标识自己;任务 `source` 记创建者。当前版本**不实现**
Bridge,只保证上述接口稳定。

## 9. 安全边界

- REST 仅 `127.0.0.1:14411`,不监听 `0.0.0.0`;不提供鉴权,因为它从来不是网络服务。
- 需要远程/多机时,由外部反向代理或 Bridge 承担鉴权,Core 不做。
- 数据目录:桌面端 = exe 同级 `data\`;独立 `serve`/`mcp` 默认 `%USERPROFILE%\.taskasion`,
  `TASKASION_DATA_DIR` 可覆盖。指向同一目录即操作同一份真相源。

## 10. 兼容红线

`todo.md` / `goals.md` 是唯一真相源;Markdown 语法不变(Python 版可读同一份文件);
写入原子(tmp + rename);外部编辑热加载;`audit.jsonl` 追加式保留;
旧 `/api/*` 路径与旧 MCP 工具签名继续可用。
