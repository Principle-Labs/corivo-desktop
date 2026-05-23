# Project 层 Spec（WorkContext 之上的月级聚合）

> 版本：draft-0 · 依赖：`work-context-merge-spec.md` 已合入 · 阻塞：`project-reconcile-spec.md`

---

## 1. 问题陈述

`/now` 现在的展示单位是 WorkContext（小时级），但用户真正的心智单位是**项目**（月级）：

- "论文截图时间选择分析" + "回顾 Corivo 记忆" + "梳理用户画像描述功能" + "time-picker 推送历史"
- 在用户眼里都是**一个项目：Corivo**
- 在 WorkContext 层看它们完全没关系（不同文件、不同 URL、不同对话）

`work_context_signals` 天花板就在这里：它只能把 WorkContext 归属到"工程名称 / 文件路径"上，抓不到**人心里的 project**。所以这一层必须走 LLM 做归属。

### 目标

- 新增 `projects` 表（月级稳定的"在做的大事"）
- 每个 WorkContext 写入时**同步**归属到一个 Project（LLM 决策）
- `/now` 页面改为按 Project 分组展示
- Stream 层先不做（见 `project-reconcile-spec.md`，v2 里补）

---

## 2. 非目标

- **不做 Stream 层**（用户调研 / 技术调研 / 代码开发）。留给下一份 spec 的 daily reconcile
- **不做 Project 手动管理**（重命名、合并、归档）。v1 全自动；手动管理视反馈再加
- **不做 Project 级的 signal 池**。归属完全依赖 LLM，不做信号预筛

---

## 3. 数据模型

### 3.1 新表 `projects`

```sql
CREATE TABLE projects (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  title           TEXT    NOT NULL,
  summary         TEXT    NOT NULL,
  category        TEXT    NOT NULL CHECK (category IN ('project','research','ops','personal')),
  status          TEXT    NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','dormant','archived')),
  started_at      TEXT    NOT NULL,
  last_active_at  TEXT    NOT NULL,
  created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
  updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_projects_status_active ON projects(status, last_active_at DESC);
```

字段说明：
- `category`：比 work_context 的三分类更粗。`project`（典型软件/产品项目）、`research`（论文阅读/调研）、`ops`（运营/沟通/事务）、`personal`（个人事务）
- `status`：`active`（有 WorkContext 在近 7 天内归属过）、`dormant`（30 天内有归属但近 7 天无）、`archived`（reconcile 认定结束或用户手动）
- 没有 `ttl_seconds`：Project 不自动 stale；状态由 reconcile 或成员 WorkContext 活跃度间接驱动（v1 里先只维护 `last_active_at`，`dormant/archived` 由下一份 spec 的 reconcile 来做）

### 3.2 `work_contexts` 新增外键

migration 同文件里追加：

```sql
ALTER TABLE work_contexts ADD COLUMN project_id INTEGER
  REFERENCES projects(id) ON DELETE SET NULL;
CREATE INDEX idx_work_contexts_project ON work_contexts(project_id, last_seen_at DESC);
```

`project_id` **nullable**。理由：
- 旧数据回填需要 LLM，跨 migration 不现实 → 留空让下一次命中时归属
- LLM 偶发失败 / 超时时允许 WorkContext 先落库、事后 reconcile 补齐

### 3.3 FTS（延后）

Project 不加 FTS。归属靠 LLM；检索用 `project.title/summary` 的 `LIKE` 足够，v1 列表页也只按 `last_active_at DESC` 排。

### 3.4 Migration 号

`012_projects.sql`（v11 是 work_contexts_fts）。

---

## 4. Project Resolver（同步归属）

新文件：`src-tauri/src/services/work_context/project_resolver.rs`

### 4.1 触发点

在 [`pipeline.rs::process_batch`](../src-tauri/src/services/work_context/pipeline.rs) 里，确定 `context_id` 之后（无论 merge 还是 create），**但在 emit `WorkContextChanged` 之前**，调一次 resolver：

```rust
let project_id = project_resolver::resolve(ctx, context_id, &draft).await.ok().flatten();
if let Some(pid) = project_id {
    if let Err(e) = ctx.work_context_repo.set_project(context_id, pid).await {
        tracing::warn!(%e, context_id, "work_context: set_project failed");
    }
}
// 再 emit WorkContextChanged
```

**关键**：resolver 失败/超时时**吞掉错误**，`project_id` 留空。WorkContext 能独立存在，不因为归属失败就阻塞主流程。

### 4.2 决策流程

```
1. 拉候选：最近 30 天 status='active' 的 projects，按 last_active_at DESC 取 top-8
2. 为每个候选拉 "近 7 个 WorkContext 标题"（按 last_seen_at DESC）
3. 调 LLM，输入 = 新 WorkContext 的 {title, summary, category} + 8 个候选 + "NEW_PROJECT" 选项
4. LLM 返回：
     {"choice": "existing", "project_id": 42}
   或
     {"choice": "new", "new_project": {"title": "...", "summary": "...", "category": "project"}}
5. existing → set_project(context_id, 42) + 更新 project.last_active_at
   new → projects.insert(new_project) → set_project(context_id, new_id)
6. 余下：不归属（LLM 判定"不属于任何已有项目、也不值得新建"，例如一次性浏览）
```

### 4.3 Prompt

新文件：`src-tauri/prompts/project_resolve.md`

```markdown
<!-- version: 1 | docs/project-layer-spec.md §4.3 -->

你是 Corivo 的项目归属员。用户刚刚被识别到一个新的"工作上下文"（短期在做的一件事）。你的任务是判断它属于**下面列出的某个长期项目**，还是**应该新建一个项目**，或者**都不是**（例如一次性浏览、不值得记录）。

## 新工作上下文

- 标题：{{wc_title}}
- 摘要：{{wc_summary}}
- 类别：{{wc_category}}

## 候选项目（共 {{n_candidates}} 个）

{{candidates_block}}

<!-- candidates_block 形如：
[1] id=42 title="Corivo 主应用" category=project last_active=2h ago
    最近在做：
      - 修 /now 页面重复 WorkContext
      - 写 project 层 spec
      - FTS 召回实现
      - 调 retrieval 评分公式
      - 对齐 distiller episode 追加
      - 整理 overview 页面"今天的事"
      - 回顾 notification 推送冷却逻辑
[2] id=37 title="GUM 论文阅读" category=research ...
-->

## 决策规则

- 如果新 WorkContext 的主题/上下文**明显延续**某个候选项目的最近工作 → `existing`，返回该 `project_id`
- 如果它看起来是一件**持续几天以上的新事情**（有明确主题、值得跟踪进度）→ `new`，给出 title / summary / category
- 如果它只是**一次性浏览、随手查资料、短暂停留**，没有长期价值 → `none`

**宁可归到已有项目也不要轻易新建**。项目是月级单位，新建的门槛应高于新建一个 WorkContext。

## 输出

仅输出 JSON，不要 Markdown 包裹：

```json
{"choice": "existing", "project_id": 42}
```
或
```json
{"choice": "new", "new_project": {"title": "...", "summary": "...", "category": "project"}}
```
或
```json
{"choice": "none"}
```
```

### 4.4 候选组装细节

```rust
pub async fn resolve(
    ctx: &DistillerCtx,
    context_id: i64,
    draft: &WorkContextDraft,
) -> Result<Option<i64>> {
    let since = Utc::now() - chrono::Duration::days(30);
    let candidates = ctx.project_repo.list_active_since(since, 8).await?;
    if candidates.is_empty() {
        // 冷启动：直接让 LLM 决定新建 or 不建（no existing）
    }
    let mut blocks = Vec::with_capacity(candidates.len());
    for (idx, p) in candidates.iter().enumerate() {
        let recent_titles = ctx
            .work_context_repo
            .recent_titles_for_project(p.id, 7)
            .await
            .unwrap_or_default();
        blocks.push(render_candidate(idx + 1, p, &recent_titles));
    }
    let prompt = render_project_resolve(RenderInput { draft, blocks });
    let decision: ResolveDecision = ctx.llm.complete_json(&LlmRequest::text(prompt)).await?;
    apply_decision(ctx, context_id, decision).await
}
```

Repo 新增方法：
- `ProjectRepo::list_active_since(since, limit) -> Vec<Project>`
- `ProjectRepo::insert(NewProject) -> Project`
- `ProjectRepo::touch_last_active(id, at)`
- `WorkContextRepo::set_project(context_id, project_id) -> Result<()>`
- `WorkContextRepo::recent_titles_for_project(project_id, limit) -> Vec<String>`

### 4.5 超时与成本

LLM 调用加 **8s 超时**（proposition pipeline 里已有 helper 可复用）。超时 → 跳过归属，下一次 reconcile 再补。

每个 batch 多一次 LLM 调用 → 按当前 batch 频率（两截图一组 propose）大致 +1 次/分钟。可接受。

---

## 5. Commands & Hooks

### 5.1 Tauri commands（`src-tauri/src/commands/`）

新文件 `project.rs`：

```rust
#[tauri::command] list_projects(status: Option<String>) -> Vec<Project>
#[tauri::command] project_by_id(id: i64) -> Option<Project>
#[tauri::command] project_work_contexts(project_id: i64, status: Option<String>) -> Vec<WorkContext>
```

在 `lib.rs::run()` 里注册。

### 5.2 前端类型（`src/lib/types.ts`）

```ts
export type ProjectStatus = "active" | "dormant" | "archived";
export type ProjectCategory = "project" | "research" | "ops" | "personal";

export interface Project {
  id: number;
  title: string;
  summary: string;
  category: ProjectCategory;
  status: ProjectStatus;
  started_at: string;
  last_active_at: string;
  created_at: string;
  updated_at: string;
}

// work_contexts 加一个字段
export interface WorkContext {
  // ...
  project_id: number | null;
}
```

### 5.3 Hooks（`src/hooks/use-projects.ts`，新文件）

```ts
useProjects(status?: ProjectStatus)         // 列表
useProjectWorkContexts(projectId: number | null)  // 某 project 下的 WorkContext
```

---

## 6. 前端变更（`/now` 页面）

### 6.1 列表结构（`src/pages/now/now-page.tsx`）

现在：

```
[ WorkContext A ]
[ WorkContext B ]
[ WorkContext C ]
```

新：

```
▼ Project: Corivo 主应用 (3)
    [ WorkContext A ]
    [ WorkContext B ]
    [ WorkContext C ]
▼ Project: GUM 论文阅读 (2)
    [ WorkContext D ]
    [ WorkContext E ]
▼ 未归属 (1)
    [ WorkContext F ]
```

- 默认按 `project.last_active_at DESC` 排序
- 每组内部按 `work_context.last_seen_at DESC` 排
- `project_id = null` 的 WorkContext 收到底部"未归属"组里
- 点击 Project 标题**可以折叠**（状态存 Zustand `ui-store`，记忆折叠偏好）

### 6.2 详情面板

WorkContext 详情面板**不变**。顶部加一行面包屑：

```
Corivo 主应用  ›  修 /now 页面重复 WorkContext
```

点击 Project 名 → 后续可扩展到 Project 页面（v1 可以只是 disabled，不跳）。

### 6.3 Overview 页面

`overview-page.tsx` 的"今天的事"卡片：

- v1 保持按 WorkContext 展示，不动
- 后续（下一份 spec 之后）可切成"今天活跃的 Projects"

---

## 7. 事件

新增事件 `ProjectTouched { project_id, kind: "created" | "updated" }`。

`WorkContextChanged` 不变；前端在收到 `WorkContextChanged` 时**同时**失效 `["projects"]` query，保证分组跟着刷。

---

## 8. 测试

### 8.1 单测

| 位置 | Case |
|---|---|
| `project_resolver` | LLM 返回 existing → `set_project` 被调 + `touch_last_active` 被调 |
| `project_resolver` | LLM 返回 new → `projects.insert` + `set_project` |
| `project_resolver` | LLM 返回 none → 两个都不调 |
| `project_resolver` | LLM 超时 → 返回 Ok(None)，不 panic |
| `project_resolver` | 零候选（冷启动）→ 候选块为空，prompt 仍然合法 |
| Repo | `list_active_since` 按 `last_active_at DESC` 排且过滤 status |
| Repo | `recent_titles_for_project` 按 `last_seen_at DESC` 取 N |

### 8.2 集成

本地回放 1 天 capture，人工验证：
- 5 个以上独立 Project 被识别
- Corivo 相关 WorkContext 80%+ 归属到同一个 Project
- 至少有一条 WorkContext 归到 `none`（说明阈值不至于把一切都新建）

---

## 9. 执行顺序

1. migration 012 + 对应 `apply_v12`
2. `ProjectRepo` trait + SQLite impl + 基础单测
3. `WorkContextRepo::set_project` + `recent_titles_for_project`
4. `project_resolver.rs` + prompt 文件 + 单测
5. 接入 `pipeline.rs`（只加 resolver 调用，不动其他）
6. Tauri commands + lib.rs 注册
7. 前端 types + hooks
8. `/now` 页面分组展示
9. 本机 dev 跑半天，肉眼验证

---

## 10. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| LLM 早期疯狂建新 Project | Project 膨胀 | prompt 里反复强调门槛；reconcile spec 会做合并/归档兜底 |
| 候选列表有偏（top-8 漏掉活跃 project） | 归属到错的 project 或误新建 | 30 天窗口 + 按 `last_active_at` 排已经覆盖常见情况；极端情况等 reconcile 修正 |
| `project_id` nullable 带来前端判空 | UI 分叉 | 一律"未归属"组兜底；空 project_id 不影响现有查询 |
| 归属错了但用户看不到改法 | 错分到老死 | v2 加手动"移动到另一个 project"；现在先看 reconcile 能不能自愈 |
| LLM 每批多一次调用的成本 | 每分钟多 1 次 | 8s 超时 + 只在新 WorkContext 或 reactivate 时跑；touch 一个活跃行不触发 |
