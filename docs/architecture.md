# Taskasion 架构 v0.1

## 组件

```
┌─ Tauri 2 壳(src-tauri, Rust)─────────────┐
│  透明/无边框/置顶/不可手动拉伸;折叠时窗口收缩为迷你条 │
│  面板半透明玻璃(HLN 玻璃令牌降 alpha,根背景放穿)  │
│  应用图标 = 托盘图标(icons/icon.ico,HLN 引擎风格) │
│  系统托盘:左键显示/隐藏;右键菜单 显示/隐藏+检查更新+退出 │
│  Ctrl+Shift+Space 全局显示/隐藏;头部 ✕ 退出  │
│  头部更新按钮 + 托盘"检查更新" → GitHub Releases 自动更新 │
│  WebView(React/TS) ──fetch──► Core REST  │
│  taskasion-core(src-tauri/src/taskasion_core/, Rust)│
│  TaskStore:todo.md 真相源 + 归一化 + 热加载 │
│  Integration API v1:REST 127.0.0.1:14411(/api/v1 与 /api)│
│  MCP server(stdio,serde_json 手写 JSON-RPC)│
│  reminder:进程内轮询 due+remind_time,内嵌提示音 │
│  Audit:audit.jsonl 追加式审计             │
└───────────────────────────────────────────┘
```

职责边界:**壳只负责渲染**,一切状态在 Core 与 todo.md。Core 与壳同进程(独立线程),仍以 `127.0.0.1` HTTP 通信——这让 Agent、脚本、未来的 Tauri 微组件可以共享同一个 Core;`Taskasion.exe serve` / `Taskasion.exe mcp` 也可独立运行。

## 分发形态:绿色便携(portable)

自用成品就是一个绿色文件夹(无安装包、无注册表、数据随身),**单 exe 即全部**:

```
release/
├─ Taskasion.exe      壳 + 内置 Core(Rust,自包含前端,~4MB)
└─ data/              todo.md · goals.md · audit.jsonl(真相源,整包拷走即迁移)
```

- 壳启动时直接在进程内起 Core 线程(绑定失败按 250ms 重试约 10s,等更新换 exe 后的端口交接;仍失败则前端复用已存在的其它实例 Core)。
- 数据目录:桌面端 = exe 同级 `data\`;`Taskasion.exe serve` / `mcp` 独立运行时默认 `%USERPROFILE%\.taskasion`,`TASKASION_DATA_DIR` 可覆盖。
- port 口径:**14411 为全项目统一端口**(api.ts 兜底、壳 CSP、Core 默认一致);旧 8737 弃用。
- 自动更新:启动 6s 后静默检查 GitHub Releases(系统 `curl.exe`),有新版时头部出现更新按钮,点击后下载 zip → 系统 `tar.exe` 解压 → 当前 exe 改名 `.old` 让位 → 新 exe 落位并重启;下次启动清理 `.old`。也可走托盘菜单"检查更新"。

## 数据契约:`todo.md`(真相源)

人、Agent、脚本都可以直接读写;Core 每次请求前检测 mtime/size 变化并重新加载(外部编辑生效,审计记为 `actor=external`)。格式:

```markdown
# Taskasion

<!-- 人可读的 Markdown checkbox;行尾 HTML 注释是机器元数据,渲染时不可见 -->

- [ ] 交季度报告 <!-- id:a1b2c3d4 due:2026-09-18 pri:p1 tags:work src:human created:2026-09-14T12:00:00 note:"附上 季度模板" -->
- [x] 买牛奶     <!-- id:e5f6a7b8 pri:p3 tags:home src:agent:mcp created:2026-09-14T09:00:00 done:2026-09-14T18:30:00 -->
```

字段:`id`(8位hex,缺省时 Core 首载自动补)、`due`(YYYY-MM-DD)、`pri`(p1-p3)、`tags`(逗号分隔)、`src`(human / agent:\<name\> / external)、`created`、`done`、`note`(备注,含空格需写成引号包裹的 `note:"多词备注"`,内部 `"` 和 `\` 转义)。保存时 Core 会归一化(补 id、按 待办→已完成 排序)。**为 Agent 写文件的建议:走 MCP/REST,别手拼注释**(手改也安全,Core 会补齐)。

提醒时刻以保留标签 `_remind:HH:MM` 存在 `tags` 里(如 `tags:work,_remind:14:30`),对外暴露为 `remind_time` 字段而不出现在普通标签中;旧版程序会把它当普通标签原样保留,因此新旧互通不丢数据。

`goals.md` 同格式(标题即目标);任务通过标签 `goal:<goalId>` 关联到目标,Core 在 `/api/goals` 返回里实时汇总每目标 `{total, done}` 进度。

## Integration API v1(REST 全部走 `http://127.0.0.1:14411/api`)

`/api/v1/*` 与 `/api/*` 是同一批路由(入口剥掉 `/v1` 后交给同一 handler),旧路径保留兼容。
完整契约见 [integration-api.md](integration-api.md)。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | /health | 存活 + 版本 + 数据目录 |
| GET | /capabilities | 能力自述(REST/MCP 入口、actor、字段、null 语义、提醒模型) |
| GET | /tasks?status=all\|todo\|done&tag=x | 列表 |
| GET | /tasks/{id} | 单个任务 |
| POST | /tasks | {title, due?, remind_time?, priority?, tags?, note?} |
| PATCH | /tasks/{id} | 部分更新(due/remind_time/note/priority 传 null 清空) |
| POST | /tasks/{id}/complete · /reopen | 勾选/回退 |
| DELETE | /tasks/{id} | 删除 |
| GET | /audit?n=50 | 审计尾部 |
| GET | /goals?status=all\|todo\|done | 目标列表(含 progress 进度统计) |
| GET | /goals/{id} | 单个目标 |
| POST | /goals | {title} |
| PATCH | /goals/{id} | 重命名 {title} |
| POST | /goals/{id}/complete · /reopen | 完成/重开 |
| DELETE | /goals/{id} | 删除 |
| GET | /plan/today | 今日规划 |

身份头:`X-Taskasion-Actor`(缺省 `human`;Agent 网关建议传 `agent:<name>`)。

## MCP 工具集(stdio,设计原则:少而稳,参考 mcp-tasks)

**与 REST 同源**:所有工具直接在同一份 Core 领域层上执行;字段与 Integration API v1 对齐
(新增 `remind_time`,清空可用 `null`,旧的 `"none"` 仍接受),前 10 个工具签名与旧 Python
FastMCP 版逐字一致。

| 工具 | 签名要点 |
|---|---|
| `task_add` | title 必填;due/remind_time/priority/tags/note 可选 |
| `task_list` | status=todo 默认;tag 过滤 |
| `task_update` | 未传字段不动;清空 due/remind_time/note/priority 传 'none' 或 null;空串视为未传 |
| `task_complete` / `task_reopen` | 勾选/回退 |
| `task_delete` | 按 id |
| `task_plan_today` | 返回 {today, overdue, today_tasks, next} |
| `goal_add` / `goal_list` | title 必填 / status=todo 默认;含每目标进度 |
| `goal_update` / `goal_complete` / `goal_reopen` / `goal_delete` | 重命名/达成/重开/删除 |
| `goal_link_task` | 给 task 追加 `goal:<id>` 标签完成关联 |
| `capabilities` | 返回 Integration API v1 能力自述 |

Agent 侧接入:Claude Code / Codex 配置 `command=<Taskasion.exe 完整路径>, args=["mcp", "--actor", "agent:codex"]`,数据目录沿用 `TASKASION_DATA_DIR`(把它指到便携文件夹的 `data\` 即操作同一份真相源)。`--actor` 只影响审计留痕(缺省 `agent:mcp`)。MCP 协议为 stdio 换行分隔 JSON-RPC 2.0,由 Core 用 serde_json 手写实现,不依赖任何第三方包。

## 提醒调度(reminder.rs,桌面壳进程内)

- 模型只有"`due` 日期 + `remind_time` 时刻,到点提醒一次":没有重复、提前 N 分钟、多级提醒、日历与设置页。
- 每 12s 轮询一次 Core 的待办列表(`list("todo")`),命中窗口 `[到点, 到点+60s)` 才响,同一任务同一分钟只响一次;
  错过的分钟不补响,已完成/已删除/无 `due`/无 `remind_time` 的任务自然不会响。
- 提示音是运行时合成的 ~1.15s 柔和和弦 WAV(内存播放,`winmm` 的 `PlaySoundW`),**内嵌在 exe 里**,
  不需要用户管理任何音频文件;同时向前端 emit `reminder-fired`,由任务行短暂高亮。
- 调度器**只读**真相源,永不写 Markdown;提醒的持久化完全由 `todo.md` 里的 `_remind:HH:MM` 保留标签承担。
- Bot / 群聊提醒不在这里:Core 不认识 QQ/OneBot/AstrBot,只对外提供 Integration API,由未来的 Bridge 自己轮询。

## 审计

`audit.jsonl`:每行 `{ts, actor, action, task_id, detail}`。REST/MCP 全部变更入库;外部直接改文件在下次加载时记 `external_edit`。这是未来"Agent 自动管理 + 人工确认"机制的地基。

## 前端 UI(HLN ui-system v2.3)

Web 端接入 HLN ui-system v2.3 设计系统(作者自研引擎;构建产物快照内置 `vendor/hln-ui-system-v2.3/`,本机存在引擎仓库时 Vite alias `@hln-ui` 直连该引擎的 `dist`):

- `main.tsx` 最先 import 引擎自包含 CSS,再加载业务样式。
- `App.tsx` 根节点挂 `data-hln-ui-root` / `data-hln-ui-version="v2.3"` / `data-hln-theme`(主题键)/ `data-hln-font="display"`。HLN root 默认铺不透明 bg-0,`styles.css` 强制 `.app[data-hln-ui-root]{background:transparent}` 放穿以实现半透明玻璃;玻璃 alpha 在 `.app` 作用域覆盖 `--hln-ui-glass` / `--hln-ui-glass-strong`。
- 控件用 `[data-hln-ui-control]`(primary/chamfer)、`[data-hln-ui-field]`、`[data-hln-ui-bar]`、`[data-hln-ui-scroll]`、`.segmented`、`.tactical-meter`;条目/面板入场用 `data-hln-motion`(item=data-stream 错峰,panel=tactical-lock);主面板与折叠迷你条均用 `data-hln-ui-no-ornament` 关闭 HLN 角饰。
- `styles.css` 只用 HLN token/变量做布局,不自带配色常量。
- 任务行交互:标题点一下就地改成输入框(Enter 存、Esc 撤、失焦存);备注平时不占位,悬停时在下方以只读一行露出,无备注则显示低对比的 `添加备注…`,点一下才进入编辑(悬停不抢焦点);行内 `◷` 就地设置/修改/清除提醒时刻。这些展开/收起只改布局与状态,不引入新的配色常量。

## 路线图

- v0.2:鼠标穿透模式、贴边折叠动画、✅ 提醒调度(due + remind_time,进程内合成提示音)
- v0.3:✅ goal(目标)对象与 task 关联;Agent 变更"待确认"队列
- v0.4:WebDAV/文件同步;多机合并策略
- 近期:Taskasion QQ Bridge(只走 Integration API v1;OneBot/AstrBot 适配留在 Bridge,Core 不引入任何群协议)
- 远期:AI 解析自然语言建任务(本地 Ollama,参考 nanoSecretary);Agent 状态灯(参考 FocuSD hooks)
