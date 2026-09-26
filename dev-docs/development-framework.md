# Taskasion 开发框架全景(v1.1.1)

> 面向"与无代码库访问权的 AI 讨论下一步开发"的自包含文档:技术栈、模块边界、数据契约、
> 对外接口、更新管线、构建发布、当前限制与扩展点,全部以源码为准(截至 2026-09-24,commit c9d7a32)。
> 面向使用者的简版在 `docs/architecture.md`;本机路径对照在 `dev-docs/dependency-map.md`(不入库)。

## 1. 产品定位与现状

Taskasion 是 Windows 桌面上的**置顶悬浮 todo/goal 小组件**,产品哲学是"人看,Agent 管":

- **本地优先**:所有状态就是两个 Markdown 文件 `todo.md` / `goals.md`(唯一真相源),人可以用任何编辑器直接改,外部修改即时生效。
- **Agent 原生**:内置 MCP server(stdio,16 个工具,带 instructions / structuredContent / annotations)供 Claude Code / Codex / ZCode 等直接接管增删改查;也可走本机 Integration API v1(REST,`/api/v1` 与历史 `/api` 同一套实现)。每一次变更(人、Agent、外部编辑)都记入追加式审计日志 `audit.jsonl`。
- **单文件即全部**:Rust core 并入 Tauri 壳同进程,发行物是一个约 3.4MB 的 `Taskasion.exe` + 同级 `data\` 目录,免安装、无注册表、删除即卸载。
- 当前:已发布 v1.1.0(2026-09-24)与 v1.1.1(2026-09-25,提醒 + Integration API v1 + 标题备注交互);v1.3.0(MCP 接入质量)开发中,计划见 `dev-docs/plan-v1.3.0.md`。数据格式与 Python 版(≤v1.0.1-aStart)双向兼容。

## 2. 总体架构

```text
┌─ Taskasion.exe(单进程)────────────────────────────────┐
│  Tauri 2 壳(main.rs,~214 行)                          │
│   · 透明无边框 320×440 置顶窗(WebView2 渲染 React 前端)│
│   · 系统托盘 / Ctrl+Shift+Space 全局快捷键 / CLI 分发   │
│                                                        │
│  WebView 前端(App.tsx ~731 行 + api.ts 63 行)          │
│   · 纯渲染 + 交互,不保存任何状态                       │
│   · 1.5s 轮询 REST;监听 update-status 事件            │
│        │ fetch http://127.0.0.1:14411/api/…            │
│        ▼                                               │
│  taskasion_core(Rust,core 线程,同进程)              │
│   · rest.rs   手写 HTTP/1.1 server(零第三方依赖)      │
│   · mcp.rs    stdio JSON-RPC 2.0(serde_json 手写)     │
│   · store.rs  todo.md / goals.md 真相源读写            │
│   · models.rs 行级解析/序列化;audit.rs 审计追加       │
│        │ 每次操作前 mtime+size 检测 → 热加载           │
│        ▼(原子写:tmp + rename)                        │
│  data\todo.md · goals.md · audit.jsonl(唯一真相源)    │
└────────────────────────────────────────────────────────┘
        Agent / 脚本从进程外通过 REST(14411)或 MCP 接入同一份真相源
```

**职责边界**(设计上的关键决定):壳只负责渲染;一切状态在 Core 与 Markdown 文件。Core 与壳同进程但走 `127.0.0.1` HTTP 通信——这让 Agent、脚本、未来可能的其它前端共享同一个 Core;`Taskasion.exe serve` / `Taskasion.exe mcp` 也能独立运行(core 是纯 Rust 库 + 可执行入口,不依赖 Tauri)。

## 3. 代码地图

| 文件 | 行数 | 职责 |
|---|---|---|
| `src-tauri/src/main.rs` | 264 | Tauri 壳入口:窗口/托盘/快捷键/CLI 分发(`serve` / `mcp --data-dir --actor` / `onboarding [--write]`)/check_update 与 apply_update 命令、core 线程启动(`serve_shared`)、reminder 线程启动、启动定位 |
| `src-tauri/src/taskasion_core/onboarding.rs` | 109 | Agent 自助接入:`onboarding` 文本与 AGENTS.md 模板的纯函数生成器 + 14411 端口探测;2 个单测 |
| `src-tauri/src/update.rs` | 413 | 自动更新全管线 + 语义化版本比较 + WinINET 代理回退;7 个单测 |
| `src-tauri/src/reminder.rs` | 409 | 提醒调度:12s 轮询、命中去重、运行时合成 WAV 提示音(winmm `PlaySoundW`)、`reminder-fired` 事件;11 个单测(7 纯函数 + 4 真文件端到端) |
| `src-tauri/src/taskasion_core/mod.rs` | 12 | 模块声明;`VERSION = env!("CARGO_PKG_VERSION")` |
| `…/models.rs` | 374 | Task 结构(含 `remind_time`);todo.md 行级 parse/render;`_remind:HH:MM` 保留标签剥离/写回;id/时间戳生成;7 个单测 |
| `…/store.rs` | 821 | TaskStore/GoalStore:mtime 热加载、归一化、原子写、全部增删改查语义;`remind_time` 校验;14 个单测 |
| `…/rest.rs` | 721 | 手写 HTTP/1.1 server;Integration API v1(`/api/v1` 与 `/api` 同一 handler)/状态码/CORS/`capabilities`;`Core` 装配结构;6 个单测 |
| `…/mcp.rs` | 1041 | stdio JSON-RPC 2.0;表驱动工具注册表(16 个工具,schema + annotations 单一事实来源);会话化协议层(instructions / actor 派生链 / notifications:message 日志 / text+structuredContent 双写 / 未知工具 -32602);空串=未传、`"none"`/null=清空;16 个单测 |
| `…/audit.rs` | 101 | 追加式审计 audit.jsonl;tail 读取;1 个单测 |
| `src/main.tsx` | 12 | 先 import HLN 引擎 CSS,再业务样式,挂 React 根 |
| `src/App.tsx` | 881 | 全部 UI:双视图(任务/目标)、输入行(含 `◷` 提醒)、标题/备注就地编辑、列表、折叠迷你条、拖拽、更新按钮、`reminder-fired` 高亮 |
| `src/api.ts` | 90 | REST 客户端(Task/Goal 类型 + 全部调用,基址 `/api/v1`);actor 固定 `human` |
| `src/styles.css` | 927 | 仅布局/放穿/动画;配色与控件外观走 HLN token |
| `src-tauri/tauri.conf.json` | — | 窗口配置(透明/置顶/跳过任务栏)、CSP、devUrl 14410、frontendDist ../dist |
| `src-tauri/.cargo/config.toml` | — | target-dir 固定到 `build/cargo-target/`,debug/test 无调试信息 |
| `vite.config.ts` | 21 | 端口 14410 strictPort;`@hln-ui` alias 直连本机引擎 dist 或 vendor 快照 |
| `scripts/sync-hln.mjs` | — | 用 `HLN_ENGINE_DIST` 环境变量把引擎 dist 刷进 vendor 快照 |

依赖面极窄:Rust 侧仅 tauri / tauri-plugin-global-shortcut / serde / serde_json / uuid / chrono,REST 与 MCP 的 HTTP 和 JSON-RPC 全部手写,零额外依赖。

## 4. 进程与线程模型

单进程 `Taskasion.exe`,几类执行流:

1. **Tauri 主线程**:窗口、托盘、全局快捷键事件。
2. **core 线程**(setup 时 spawn):`rest::serve("127.0.0.1", 14411, data_dir, 40)` 阻塞监听;绑定失败每 250ms 重试,最多 40 次(约 10s)——为"自动更新换 exe 后等旧实例释放端口"设计;仍失败则放弃,**前端会自动落到已存在的其它实例的 core 上**(多开语义)。
3. **每连接线程**:每个 HTTP 请求一个线程,处理完即断(Connection: close)。
4. **更新线程**:启动 6s 后静默检查一次(`schedule_auto_check`);检查/下载/换 exe 都在独立线程,通过 `update-status` 事件向前端推送状态。

数据并发模型:每个真相源文件一个 `MdStore`(内存 items + Mutex),REST/MCP 的每个公开操作都先 `reload_if_changed`(检测 mtime+size)再操作,取值-改字段-保存全程持锁;无跨文件事务(tasks 与 goals 各自独立加锁)。

## 5. 数据层:真相源契约(最核心的兼容性承诺)

### 5.1 todo.md 行格式

```markdown
- [ ] 交季度报告 <!-- id:a1b2c3d4 due:2026-09-18 pri:p1 tags:work,urgent src:human created:2026-09-14T12:00:00 note:"附上 季度模板" -->
- [x] 买牛奶     <!-- id:e5f6a7b8 done:2026-09-14T18:30:00 … -->
```

精确文法(与 Python 版逐行等价,models.rs):

- 行:可选缩进 + `- [` + 空格或 `x`/`X` + `] ` + 标题 + 可选行尾 `<!-- … -->` 元数据;非 checkbox 行一律忽略(标题行、普通文本都安全)。
- 元数据区间:从**第一个** `<!--` 到其后**最后一个** `-->`,且 `-->` 后只允许空白。
- 键值:空格分隔 `k:v`,仅认 8 个键 `id/due/pri/tags/src/created/done/note`;未知键忽略;同名键**后写覆盖先写**(对齐 Python dict 语义)。
- `tags` 里的 `_remind:HH:MM` 是**内部保留标签**:解析时被拎出来放进 Task 的 `remind_time` 字段、从 `tags` 中移除(非法值直接丢弃),渲染时再追加回 `tags`。所以它不在上面 8 个键里,也不会出现在普通标签、REST、MCP 或 UI 中。
- `note` 是唯一允许空格的值:引号包裹 `note:"多词备注"`,内部 `"` 与 `\` 转义(`\x` 保留 x);引号未闭合时整段回退普通 token 解析。其余值不允许空格(`tags` 用逗号分隔)。
- `id`:8 位小写 hex(uuid4 simple 前 8 位);`created`/`done`:本地时间秒级 ISO(`%Y-%m-%dT%H:%M:%S`)。
- 解析容错:标题空 → `(未命名)`;`src` 缺省 → `human`。

### 5.2 归一化与原子写(保存语义)

- **首载与外部重载**:缺 id / id 重复 → 现场分配新 id;缺 created → 补 now;发生补齐即原子回写(外部手写行自动获得稳定 id)。
- **保存顺序**:待办在前、已完成在后(组内保持原顺序);新任务 `insert(0)` 插到最前。
- **原子写**:写 `<file>.md.tmp` 后 rename;`*.md.tmp` 在 .gitignore。
- 文件头是固定注释引导(TODO_HEADER / GOALS_HEADER),保存时重建。

### 5.3 热加载语义

每次公开操作(list/add/update/…)前 stat(mtime+size),与上次不同则重新解析整个文件;新增条目(与内存 id 集合的差集)记审计 `actor=external, action=external_edit`。因此:人用任何编辑器改文件 → 下一次 REST 轮询(≤1.5s)即生效。无文件系统 watcher,检测粒度是"每次操作前"。

### 5.4 goals.md 与目标关联

- goals.md 行格式与 todo.md 完全相同(复用同一 Task 结构与解析器;goal 实际只用 title/done/created 等基础字段)。
- 关联方式:任务打标签 `goal:<goalId>`;`GET /api/goals` 返回时实时统计该标签任务 `{progress:{total,done}}`。删除目标不级联处理任务标签。
- 审计动作前缀 `goal_*`(goal_add / goal_rename / goal_complete / goal_reopen / goal_delete;外部编辑记 `external_edit_goals`)。

### 5.5 audit.jsonl

每行一个 JSON 对象,字段顺序固定:`{"ts":"…","actor":"…","action":"…","task_id":"…"|null,"detail":"…"}`。actor 取值:`human`(REST 缺省)、`agent:mcp`(MCP 缺省)、`external`(检测到外部文件编辑)、`scheduler`,或调用方自定义的 `X-Taskasion-Actor` 头 / `mcp --actor` 值(如 `agent:claude`、`bot:qq`)。Core 不解析这些值的含义,只原样留痕。追加写,Mutex 保护单写者;`GET /api/audit?n=` 读尾部。

## 6. REST API(127.0.0.1:14411,仅本机回环)

手写 HTTP/1.1(rest.rs,零依赖):Content-Length body、CORS 全开(`*`,方法 GET/POST/PATCH/DELETE/OPTIONS,头 Content-Type + X-Taskasion-Actor)、每个响应 `Connection: close`、30s 读超时。**没有**鉴权、keep-alive、chunked、压缩——任何本机进程都能改数据,这是接受的威胁面(SECURITY.md 有说明)。

**Integration API v1**:入口处 `normalize_path` 把 `/api/v1/xxx` 剥成 `/api/xxx`,交给完全相同的 handler,因此两套路径不可能出现两套语义;`/api/*` 是历史路径别名。`GET /api/capabilities`(MCP 的 `capabilities` 工具复用同一函数)对外自描述字段、actor、null 语义与提醒模型。

| 方法 | 路径 | 请求 | 成功响应 |
|---|---|---|---|
| GET | /api/health | — | `{ok:true, version, data_dir}` |
| GET | /api/capabilities | — | 能力自述(api_version/transports/actors/task_fields/null_semantics/reminder) |
| GET | /api/tasks?status=all\|todo\|done&tag=x | — | Task[] |
| GET | /api/tasks/{id} | — | 200 Task(不存在 404) |
| POST | /api/tasks | {title, due?, remind_time?, priority?, tags?, note?} | 201 Task |
| PATCH | /api/tasks/{id} | {title?, due?, remind_time?, priority?, tags?, source?, note?} | 200 Task |
| POST | /api/tasks/{id}/complete · /reopen | — | 200 Task |
| DELETE | /api/tasks/{id} | — | 204 |
| GET | /api/audit?n=50 | — | 审计条目数组(新→旧) |
| GET | /api/plan/today | — | `{today, overdue, today_tasks, next(≤5)}` |
| GET | /api/goals?status=all\|todo\|done | — | Goal[](每项附 `progress{total,done}`) |
| GET | /api/goals/{id} | — | 200 Goal |
| POST | /api/goals | {title} | 201 Goal |
| PATCH | /api/goals/{id} | {title} | 200 Goal |
| POST | /api/goals/{id}/complete · /reopen | — | 200 Goal |
| DELETE | /api/goals/{id} | — | 204 |

语义细节(讨论接口演进时要知道的现状):

- 错误统一 `{"error":"…"}`:CoreError::NotFound→404、Invalid→400、Internal→500;未匹配路由 404 `{"error":"not_found"}`;OPTIONS→204;**body JSON 解析失败→400**。
- PATCH 字段级校验:title/source 须字符串,due/remind_time/priority/note 须字符串或 **null(清空)**,tags 须字符串数组;非法类型字段被静默跳过;一个有效字段都没有 → 原样返回当前任务(旧版行为,不报错、不写文件、不记审计)。
- 空字符串与 null 等价于清空(`clear_or_keep`);`remind_time` 额外做格式校验(非法 → 400),且"先整体校验、后统一落库",不会留下半截改动。
- `/api/audit` 的 `n` 解析失败返回 500(刻意对齐 Python int() 行为的历史怪癖)。
- plan_today:overdue = due<今天;today = due=今天;next = due>今天按 (due, priority) 升序取前 5。
- 身份:`X-Taskasion-Actor` 头,缺省 `human`;仅作审计标识,无鉴权作用。

## 7. MCP server(stdio)

独立运行:`Taskasion.exe mcp [--data-dir <dir>] [--actor <name>]`(无 Tauri,挂回父终端;数据目录缺省 `%USERPROFILE%\.taskasion`,`TASKASION_DATA_DIR` 覆盖)。协议是 **newline-delimited JSON-RPC 2.0**(serde_json 手写):

- `initialize` → 回显客户端 protocolVersion + `serverInfo{name:"taskasion",version}` + `instructions` 使用说明,声明 `capabilities`(tools+logging);`ping` → `{}`;`tools/list` → 16 个工具(带 annotations);`tools/call` → 结果 **text + structuredContent 双写**(同一个 JSON 值;业务错误 isError=true 且 `structuredContent.error` 带错误码 `not_found`/`invalid`/`internal`);未知工具 → 协议错误 **-32602**;通知(无 id)不回包;未知方法 → -32601;日志经 `notifications/message` 同步(stderr 保留)。
- actor 解析顺序:`--actor` 显式 > env `TASKASION_MCP_ACTOR` > `clientInfo.name` 派生(如 `agent:mcp:codex`,名字清洗为字母数字与 `-_.`) > 缺省 `agent:mcp`;MCP 与 REST 调的是同一批领域函数。

| 工具 | 参数 | 备注 |
|---|---|---|
| task_add | title 必填;due(YYYY-MM-DD)/remind_time(HH:MM)/priority(p1-p3)/tags[]/note;goal_id 可选(创建即关联) | |
| task_list | status=todo 缺省(all/done);tag 过滤;query 按标题/备注子串过滤(大小写不敏感) | |
| task_update | task_id 必填;未传字段不动;**清空 due/remind_time/note/priority 传字符串 'none' 或 null**;**空串=未传(老 Agent 习惯)** | |
| task_complete / task_reopen / task_delete | task_id | |
| task_plan_today | 无 | 同 REST plan_today |
| goal_add | title | |
| goal_list | status=todo 缺省 | 含实时 progress |
| goal_link_task / goal_unlink_task | goal_id + task_id | 追加/移除 `goal:<id>` 标签(均幂等) |
| goal_update / goal_complete / goal_reopen / goal_delete | goal_id(+title) | 与 REST 对等 |
| capabilities | 无 | 返回 Integration API v1 能力自述(与 `/api/v1/capabilities` 同一份) |

annotations 约定:`task_list`/`task_plan_today`/`goal_list`/`capabilities` 只读;`task_delete`/`goal_delete` 危险;完成/回退/关联类幂等;全部 `openWorldHint:false`。

MCP 无 resources/prompts/sampling/进度通知;桌面壳运行时,MCP 子进程与壳**通过同一个文件**(真相源)间接同步,不共享内存。

## 7.1 提醒调度(reminder.rs,仅桌面壳进程内)

- **模型**:只有 `due`(YYYY-MM-DD)+ `remind_time`(HH:MM),"到点响一次"。没有重复、提前 N 分钟、多级提醒、日历、设置页;`due` 仍是纯日期,`plan_today` 语义不变。
- **持久化**:提醒时刻不新建状态源,而是以保留标签 `_remind:HH:MM` 存在 `todo.md` 的 `tags` 里(如 `tags:work,_remind:14:30`)。Core 解析时剥离为外部字段 `remind_time`,渲染时写回;该标签永不出现在 `tags`、REST、MCP 或 UI 中。旧版程序会把它当普通标签原样保留 → 新旧互通不丢数据。
- **循环**:`spawn(core, app_handle)` 起一条线程,每 12s `store.list("todo", None)`;`hits_from` 命中条件为 `0 <= (到点 - now) < 60s`(错过的分钟不补响),已完成/删除/无日期/无合法时刻的任务天然不会命中。同一 `id@due time` 在 24h 内只响一次(进程内 `HashMap`,重启后同一分钟内可能再响一次,已接受)。
- **提示音**:运行时合成 ~1.15s 的 44.1kHz/16bit 单声道 WAV(880/1320/1760/2637Hz 泛音 + 指数衰减,8ms 起音、20ms 收尾淡出),缓存在 `static OnceLock<Vec<u8>>`,用 `winmm` 的 `PlaySoundW(..., SND_MEMORY|SND_ASYNC|SND_NODEFAULT)` 播放——不加任何 crate,不需要用户管理音频文件。
- **前端联动**:同时 `app.emit("reminder-fired", {id,title,due,remind_time})`,`App.tsx` 收到后把该任务行 `data-fired` 高亮 5s。
- **边界**:调度器只读真相源,永不写 Markdown;Bot/群聊提醒不在这里——Core 不认识 QQ/OneBot/AstrBot,只对外提供 Integration API,由未来的 Bridge 自己轮询。

## 8. 桌面壳(main.rs)

- **窗口**:320×440 逻辑尺寸,`decorations:false` + `transparent:true` + `shadow:false` + `alwaysOnTop` + `skipTaskbar` + 不可手动 resize。`additionalBrowserArgs` 带 `--proxy-bypass-list=<-loopback>;…`——防止 WebView2 把 127.0.0.1 的 core 请求发给系统代理。
- **CSP**:`connect-src` 仅允许 self 与 `http://127.0.0.1:14411`(dev 另加 14410/vite ws)。
- **托盘**:左键单击=显示/隐藏;右键菜单 显示/隐藏 + 检查更新 + 退出;tooltip 带快捷键提示。
- **全局快捷键**:`Ctrl+Shift+Space` 显隐(tauri-plugin-global-shortcut)。
- **启动**:core 线程 → 清理 `.exe.old` → 安排 6s 后静默检查更新 → 停靠主屏右上角(x = 屏宽 - 窗宽 - 28,y = 110)。
- **头部 ✕ / 托盘退出** = 退出整个进程(core 同进程,无子进程回收问题)。
- **CLI 分发**(main.rs `run_cli`):`Taskasion.exe serve [--host --port --data-dir]` 起 REST;`Taskasion.exe mcp [--data-dir DIR] [--actor NAME]` 起 MCP(actor 缺省 `agent:mcp`);release 是 windows 子系统,子命令先 `AttachConsole(ATTACH_PARENT_PROCESS)` 让 stdout 可见。
- **提醒线程**:`setup()` 里 `Arc<Core>` 先交给 REST 线程(`serve_shared`),再 `reminder::spawn(core, app.handle().clone())` —— 壳与 REST 共享同一个 Core 实例,提醒不需要再起一份。

## 9. 前端

- **状态模型**(App.tsx):`tasks/goals/view(tasks|goals)/title/due/pri/note/goalTitle/filter(today|all)/showPending+showDoneToday+showArchive+showDone(四段折叠,各自 localStorage 持久)/collapsed/offline/openGoalId/editingNoteId/editingTitleId/editingRemindId/firedId/upd(更新状态)`。无路由、无全局 store、无状态库。时刻选择用 WheelTime 纯文本组件(滚轮 ±5 分钟,提交时规范化;不用原生 type=time,规避小窗内的原生弹层)。
- **数据获取**:`useEffect` 每 1500ms 轮询 `fetchTasks()+fetchGoals()`;失败置 `offline`(头部 OFFLINE 灯)。所有变更操作后手动 `refresh()`。核心数据**不靠事件推送**;唯一的 Tauri 事件是 `update-status`(自动更新)与 `reminder-fired`(任务行 5s 高亮)。
- **任务行交互**:标题点击→就地输入框(Enter 存 / Esc 撤 / 失焦存,`stopPropagation` 防止冒泡到行的"打开备注");备注默认不占位,悬停时在行下方以只读一行露出,无备注则显示低对比 `添加备注…`(悬停不抢焦点),点击才进入编辑;编辑中鼠标移出不会收起;内容清空即删除备注。`noteClosed`/`titleClosed`/`cancelEdit` 三个 ref 用来避免"失焦保存"与"紧接着的点击"互相打架。
- **提醒入口**:新建区 `◷` 按钮展开 `type="time"` 输入(不占常驻空间);已有任务在行内用 `◷` / `◷ 14:30` 就地设置、修改或清除(无日期时自动绑定今天,时刻已过则绑到明天)。
- **视图**:任务/目标双 tab;`filter=today` 时只显示 无 due 或 due≤今天 的待办;目标视图可下钻某目标(按 `goal:<id>` 标签过滤,含进度条 tactical-meter)。
- **窗口折叠**:展开 320×440 ↔ 迷你条 320×52,`getCurrentWindow().setSize(LogicalSize)`;两种状态的上栏等高(52px)。
- **头部手势**:pointer 手势——按下记录,移动 >6px 判定拖动(`startDragging()`),否则抬起判定单击收起;按钮上的按下不参与(替代 data-tauri-drag-region 以免两种手势互抢)。
- **长标题跑马灯**:溢出时整段来回巡航(测量文本宽度决定距离),悬停暂停。
- **更新按钮**:监听 `update-status` 事件渲染状态字符(available=⤓ 可点触发 `invoke("apply_update")`;downloading=···;latest=✓ / error=! 4 秒后自动消失)。
- **HLN 集成**(架构约束,改动 UI 前必读):
  - `main.tsx` 先 import `@hln-ui/hln-ui-system-v2.3.css`(引擎一体包),再 `./styles.css`。
  - App 根节点挂 `data-hln-ui-root` / `data-hln-ui-version="v2.3"` / `data-hln-theme` / `data-hln-font="display"`。
  - 引擎 root 默认铺不透明 bg-0 → `styles.css` 强制 `.app[data-hln-ui-root]{background:transparent}` 放穿;玻璃 alpha 在 `.app` 作用域覆盖 `--hln-ui-glass` / `--hln-ui-glass-strong`。
  - 控件用引擎属性变体:`[data-hln-ui-control]`(primary/chamfer)、`[data-hln-ui-field]`、`[data-hln-ui-bar]`、`[data-hln-ui-scroll]`、`.segmented`、`.tactical-meter`;入场动画 `data-hln-motion`(item=data-stream 错峰,panel=tactical-lock);主面板与迷你条都 `data-hln-ui-no-ornament` 关角饰。
  - **styles.css 只写布局**;配色/质感一律 HLN token,不自带颜色常量。

## 10. 自动更新(update.rs,零新增依赖)

1. **检查**:`GET api.github.com/repos/AbsoluteHLN/Taskasion/releases/latest`(系统 curl.exe,15s 超时)→ 直连失败读注册表 WinINET 代理(ProxyEnable/ProxyServer)再试一次;取 `tag_name`(去 v 前缀)与资产(**优先 `*-portable.zip`** 后缀匹配,否则任意 `.zip`,不挑 Source code)。
2. **比较**:完整语义化版本比较(核心三元组 → 正式版>预发布 → 预发布逐段,数字段<字母段;beta.2<beta.3<beta.10<rc.1<1.1.0)。不比当前新 → `latest` 状态(带线上版本号)。
3. **提示**:有新版 → 事件 `available`,头部出现 ⤓ 按钮;用户点击(或托盘触发检查后点击)→ `apply`。
4. **安装**:下载 zip(600s 超时,同样直连失败走代理)→ `%TEMP%\taskasion-update-<pid>\` → `tar -xf` 解压 → 递归找 `Taskasion.exe`(大小写不敏感,兼容历史小写名)→ 运行中 exe 改名 `.exe.old` 让位 → 新 exe 复制落位(用包内自己的文件名,旧安装借此迁移大小写)→ spawn 新进程 → 旧进程 exit(0)。任一步失败回滚(恢复改名)保证旧版仍能启动。
5. 下次启动 `cleanup_old()` 删残留 `.exe.old`。
6. 已知环境事实:api.github.com 直连可通;release 资产(objects.githubusercontent.com)直连常被重置——正是代理回退存在的原因。

## 11. 构建、测试与发布流程

### 构建

```bash
# 前端(本机注意:node_modules 是 junction,不要让 pnpm 碰它,直接调 vite)
node_modules/.bin/vite build          # 或 pnpm build(有触发 junction 清空的风险,见 dependency-map.md)

# 测试(65 个单测:store 14 / models 7 / update 7 / rest 7 / mcp 16 / reminder 11 / onboarding 2 / audit 1)
cd src-tauri && cargo test

# 桌面 exe(先杀运行中的实例,否则 os error 5)
Get-Process -Name Taskasion -ErrorAction SilentlyContinue | Stop-Process -Force   # taskkill //F 在 PowerShell 下报参数错误
cd src-tauri && cargo build --release
# 产物:E:\Projects\Taskasion\build\cargo-target\release\Taskasion.exe(同级 data\ 是用户数据)
```

release profile:`opt-level="s"`、lto、codegen-units=1、strip、panic=abort(约 3.4MB)。

### 端到端验证脚本(不提交,只在 temp/ 下)

对 `build/cargo-target/release/Taskasion.exe` 起真进程跑,证据落在 `verify-evidence/`:

```powershell
powershell -File temp/verify-integration-api.ps1 -Exe <exe> -DataDir temp/verify-data-rest  -Out verify-evidence/integration-api-rest.txt  # 49 断言:REST v1 vs 旧路径/参数语义/热加载/回环/审计
powershell -File temp/verify-mcp.ps1             -Exe <exe> -DataDir temp/verify-data-mcp   -Out verify-evidence/mcp-stdio.txt            # 31 断言:15 工具/协议/参数兼容/审计 actor
powershell -File temp/verify-legacy-compat.ps1   -Exe <exe> -DataDir temp/verify-data-legacy -Out verify-evidence/legacy-compat.txt       # 45 断言:Python 时代 todo.md/goals.md 读入-改写-重启不丢数据
```

踩过的两个坑(下次直接用对):**① 脚本必须带 UTF-8 BOM** —— 本机 `pwsh` 实际是 Windows PowerShell 5.1,
默认 GBK 读 BOM-less UTF-8 脚本会把中文 payload 变成乱码并解析失败;**② 读 markdown/JSON 一律加
`-Encoding UTF8`**,否则读到 ANSI 乱码会伪造出"文件被写坏"的假失败。另外 `cmd.exe /c "... < in > out"`
的引号会被 PowerShell 重新拼接而卡死,脚本里改用 `System.Diagnostics.Process` 直接重定向 stdio。

### 发布(必须用户明说"发布"才执行 commit/push)

1. 版本号三处 + Cargo.lock 同步 bump:`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`。
2. `vite build` → taskkill → `cargo build --release`。
3. PowerShell `Compress-Archive` 打 zip(仅一个 Taskasion.exe 在 zip 根;命名 `Taskasion-<ver>-portable.zip`)。**不用 Git Bash tar.exe(中文编码已坏)**。
4. 扫敏感信息:exe 内不得出现 QQ 号/邮箱/本机用户名/代理端口等(`E:\dependency-cache` 路径字符串是历来既有状态)。
5. commit 信息简短(如 `v1.1.0: 正式版`)→ push。
6. `gh release create v<ver> <zip> --title "Taskasion v<ver>" --notes-file build/release-notes.md --latest`(tag 由 gh 在远端创建,本地不用打 tag;非 prerelease)。

### 铁律(发布/开发红线)

- 用户说"发布"前绝不 commit/push;commit 信息简短。
- 永不 `cargo clean`;永不删改 `release\data\` 用户数据;永不 cargo build 进带 `&&` 的后台链。
- 依赖全部取自 `E:\dependency-cache`;node_modules 是 junction,禁止单独安装/联网安装;不让 pnpm 清 modules 目录。
- 端口 14410/14411 全项目统一,不可改。
- 样式只写布局,token/variant 交给 HLN 引擎。
- 数据格式与 Python 版双向兼容是硬承诺(models.rs 注释明确"行为逐行等价")。

## 12. 当前限制与扩展点(供"下一步开发"讨论)

### 架构层

- **无事件推送**:前端 1.5s 轮询;core 无 SSE/WebSocket。做实时协作前需要决定推送通道(提醒走的是壳进程内的 Tauri 事件,不经过 core)。
- **无文件系统 watcher**:热加载靠操作前 stat(mtime+size),同秒内 size 不变的精确外部编辑理论上可能漏检(mtime 粒度问题);审计里外部编辑只能记"新增了哪些 id",不记删除/修改的 diff。
- **REST server 极简**:无鉴权(本机任何进程可写)、无 keep-alive、无并发安全的多请求事务;`/api/audit` n 非法返回 500 的怪癖是有意保留的 Python 兼容。
- **多实例语义**:第二个实例 core 绑定失败后直接放弃,前端复用第一个实例的 core——若两者数据目录不同, UI 显示的是"别人的"数据,无提示。
- **MCP 面窄**:无 resources/prompts/进度通知;MCP 子进程与桌面壳只通过文件同步(无跨进程事件)。
- **提醒去重是进程内状态**:同一任务同一分钟内重启会再响一次(可接受);提醒窗口只有 60s,休眠/关机会错过,不补响;提示音是单次柔和和弦,不是持续闹铃。
- **Bot 接入只有接口没有实现**:Core 不认识 QQ/OneBot/AstrBot,`bot:qq` 之类的 actor 只是约定字符串;真正的 QQ Bridge 尚未存在(见 docs/integration-api.md §8)。
- **goals 复用 Task 结构**:goal 的 due/priority/tags/note 字段存在但 UI 不暴露;goal 无层级(无 sub-goal)。

### 功能层(= 现有路线图缺口,出自 docs/architecture.md + 现状盘点)

- 鼠标穿透模式(点击穿透到桌面)——未做
- 贴边折叠/停靠动画——未做
- **提醒调度——已做**(1.2 开发线:`due` + `remind_time`,`reminder.rs` 进程内轮询 + 内嵌合成提示音 + 任务行高亮;没有重复/提前/日历/设置页)
- **Integration API v1 + MCP 对齐——已做**(`/api/v1` 与 `/api` 同一 handler;MCP 工具补齐到 15 个并复用同一领域层;`capabilities` 自述)
- **Taskasion QQ Bridge——接口已就位、实现未做**(只准走 Integration API v1)
- Agent 变更"待确认"队列(审计日志已是地基,但还没有 UI/流程)——未做
- WebDAV/文件同步、多机合并策略——未做
- 自然语言建任务(本地 Ollama)——未做
- Agent 状态灯——未做
- 窗口位置/折叠状态**不持久化**(每次启动回右上角展开态)
- 单一内置主题(arknights),无设置面板;无全局设置存储(现在完全没有 config 文件)
- 审计无轮转/清理;`GET /api/audit` 全文件读取后取尾(量大后变慢)
- 图标/安装包仅便携 zip;tauri bundle 配了 nsis target 但实际发布从未用过

### 已知平台细节

- 150% DPI:清晰度与逐像素对齐已修(1.1.0 交付),后续 UI 改动要维持这套像素级校准(styles.css 有大量注释)。
- WebView2 `additionalBrowserArgs` 的 proxy-bypass 不可去掉,否则 core 请求会进系统代理。
- 自动更新的"运行中 exe 只能改名不能覆盖"是 Windows 约束,`.exe.old` 清理在启动时做。

## 13. 下一步候选方向(带优先级直觉,供讨论)

1. **设置持久化**:第一个 config(窗口位置/折叠态/主题/轮询间隔),顺带决定配置文件放 data\ 还是独立。提醒刻意没有设置页,别顺手加回来。
2. **推送通道**:core 加 SSE(手写 HTTP 上加一个 streaming 端点成本低)替代轮询,是做实时协作/多 UI 的前置。
3. **Agent 待确认队列**:审计已有 actor 数据,加"待确认"状态与 UI 流程,把"Agent 管"从直接生效升级为可审计可拦截。
4. **Taskasion QQ Bridge**:独立进程,只调 Integration API v1(命令解析、鉴权、限流、OneBot/AstrBot 协议全在 Bridge 里);Core 保持不懂群协议。
5. **外部编辑检测强化**:换 ReadDirectoryChangesW watcher(Windows API,仍可零第三方),外部编辑审计能记 diff。

(以上是素材整理,不是决定;具体取舍与设计需讨论。)
