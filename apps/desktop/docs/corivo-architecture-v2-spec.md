# Task-Centric + Tags + About-User Spec

> 版本：draft-2（重大方向调整 — 取代 draft-1 的 kind 路线）
> 关系：在 `event-task-project-spec.md` 三层结构上做**结构性改造**，不再是增量叠加
> 状态：设计，尚未落代码 —— **在你点头之前，不改一行代码**
> 配套：[design-decisions.md](design-decisions.md)、[roadmap-validate-reflect-automate.md](roadmap-validate-reflect-automate.md)
>
> 核心判断（draft-2 修订）：
> - **events 是 Single Source of Truth**。task 是物化的语义投影、time_segments 是派生的时间投影、about_user 是**观测派生的闭环反思层**（既从 events / overrides 蒸馏产出，又注入回上游 LLM prompt 当 prior — 来自观测，反过来指导观测）
> - **task 升级成骨架，project 降级成可选标签**。所有"一件事"——大到"重构 attribution"、小到"订外卖"——都是平等的 task。task 不再依赖 project 存在
> - **tag 用 namespace 区分语义**：`project` / `domain` / `topic` / ...。一个 task 可挂 0/1/N 个 tag。namespace 自然承载工作/生活/主题等分类
> - **kind 字段不做了**。draft-1 里的 focused_work / context_work / leisure 三分类被 tag namespace 更优雅地覆盖（"挂 project tag" ≈ 工作；"挂 domain:吃饭 tag" ≈ 生活；什么都没挂 ≈ 未分类）
> - **多 active task 并行是常态**（订外卖期间也在写代码）。attribution 不是单选,是"找到合适的 active task 并入 / 还是开新 task"

---

## 1. 问题陈述

### 1.1 当前 P/T/E 架构留下的两个洞

**洞 1：所有 task 都被假定"该有一个 project"。**

`event_attributor` 的隐含语义是：每个 event 都该归属到某 task,每个 task 都该挂在某 project 下。这导致：

- 用户摸鱼（看 YouTube）、个人事务（订外卖）、探索性活动（刷 HackerNews 找灵感）产生的 events 永远卡在"待分类"
- `retrospective_reviewer` 反复尝试给这些 events 找 project 归属,浪费 LLM 调用
- "一件完整的事"语义没地方放——"订外卖" 既不是 Corivo project 的子任务,又不该是孤儿 events

**洞 2：只有"项目维度"一种叙事。**

`task_narrator` 和 `project_rollup` 都是项目维度的聚合。用户想"按时间线看一天" / "那段下午我都在做什么"——没有叙事单元。

### 1.2 draft-1 的 kind 路线为什么被否决

draft-1 提议在 events 加 `kind` 字段（focused_work / context_work / leisure）。但讨论中发现：

- **Event 是原子单位，不应该承载分类语义**（最根本的反对理由）。分类是聚合层的事 —— 你不会给"一次按键"分类，会给"一段写文章的过程"分类。同理 event 是"屏幕一段连续活动"的原子记录，分类应当由它聚合成的 task 承担（通过 task 是否挂 tag 表达）。把 kind 加到 event 是 **类型错位** —— 把聚合层概念下放到原子层
- kind 三分类是闭集，边界 case 多（学习 / 个人事务 / 探索性浏览算啥）
- 同一 task 内 events 的 kind 高度一致——把 kind 放 event 层会反复在同 task 内做同样判断，浪费 LLM 调用
- kind 解决的是"摸鱼 events 没地方放"的症状，但根因是"task 必须挂 project"的结构性约束
- 真正的解法不是给 event 加分类，是让 **task 脱离 project** 独立存在，把 project 降为可选标签

draft-2 直接砍掉 kind 字段。"工作/休闲" 的语义由 **task 是否挂 namespace='project' 的 tag** 自然表达；不挂任何 tag 的 task 是"其他"分类的合法终态（不是失败状态）。

### 1.3 新架构概览

主管线（observations → events → tasks → tags / time_segments）：

```
                        ┌──────────────────┐
                        │  observations    │
                        └────────┬─────────┘
                                 ▼
                        ┌──────────────────┐
                        │     events       │  ★ Single Source of Truth
                        │   + facts        │     (无 kind 字段)
                        └────────┬─────────┘
                                 │
                                 ▼ task-first attribution
                        ┌──────────────────┐
                        │      tasks       │  ★ 骨架,任意粒度,多 active 并行
                        │  state ∈         │     state: {active, done, dropped}
                        │   {active,       │     paused 派生(view 算)
                        │    done,         │
                        │    dropped}      │
                        └──┬─────┬─────┬───┘
                           │     │     │
        ┌──────────────────┘     │     └──────────────────┐
        ▼                        ▼                         ▼
┌────────────────┐    ┌──────────────────┐    ┌──────────────────┐
│ task_anchors   │    │ task_tags ↔ tags │    │ time_segments    │
│ (跟随 task,    │    │  (软分类)         │    │ (时间投影,派生)  │
│  CASCADE)      │    │  namespace 区分:  │    │                  │
└────────────────┘    │   project/domain/ │    └──────────────────┘
                      │   topic/...       │
                      └────┬──────────────┘
                           │
                           ▼
                  ┌────────────────┐
                  │  tag_anchors   │
                  │ (跟随 tag,     │
                  │  CASCADE)      │
                  └────────────────┘
```

闭环反思层（about_user · 来自观测，反过来指导观测）：

```
   ╔══════════════════════════════════════════════════╗
   ║   about_user · 闭环反思层                         ║
   ║                                                  ║
   ║   ⬆ distill (user_profile_distill pass)           ║
   ║      from events + user_overrides                ║
   ║                                                  ║
   ║   ⬇ inject as prior                               ║
   ║      → event_boundary / task_attribute           ║
   ║      → tag_assign / retrospective_reassign       ║
   ╚══════════════════════════════════════════════════╝
```

四个独立维度，各管一摊：

- **tasks**：任意粒度的"一件事"，独立存在
- **tags**：可选软分类，namespace 区分语义；挂 0 个 tag 是合法终态（"其他"）
- **tag_anchors / task_anchors**:两张独立表分别跟随 tag / task 生命周期(各 ON DELETE CASCADE),不做 polymorphic FK
- **about_user**：观测派生的闭环反思层（从 events + overrides 蒸馏产出，注入回所有上游 LLM prompt 当 prior）—— 它**不是**凭空的配置，是被持续生长出来的知识

### 1.4 spec → vision 物理对应表

本 spec 不是为"漂亮地组织数据"而设计，是为 [vision-public.md](vision-public.md) 的 **AI 同事 5 阶关系**逐阶提供物理基础。下表锁定每条 vision 句子对应的 spec 承载者，避免 spec 演化时漂离 vision，也避免 vision 演化时 spec 不知道哪里要跟改。

| Vision 阶段 | Vision 原文 | spec 物理承载 |
|---|---|---|
| **一·看着** | "持续观察你的工作流" | observations + events SSOT + facts |
| **二·照见** | "按时间维度，按项目维度，都能让你看清真实的一天" | §8.1 view switch（Project 子视图 / Timeline 子视图） |
| **二·照见** | "今天在 X 上花了 N 小时，其中真正写代码 M 小时" | tasks + events 时长统计 + `get_namespace_breakdown` command |
| **二·照见** | "本周 12 小时投入 X 项目，进展信号只增加 1%" | tasks + facts (进展信号);**完整支撑还需要 task progression metric — Open Question** |
| **二·照见** | "今天第 4 次切回这个 bug，加起来已经 2 小时" | retrospective.task_split 识别切换 + tasks 累计时长 |
| **三·守住** | "你说今天要做什么，它帮你不漂走" | **当前 spec 缺 user-stated intent 一等公民**。最低成本扩展:about_user 加 `kind=daily_intent, scope=date` 临时条目;不需要新表 |
| **四·起草** | "草稿、清单、回复，它准备好等你审" | spec 不直接做。但 about_user.work_habit + task_pattern 是起草 agent 必须的 prior — 物理基础已铺好 |
| **五·代办** | "例行重复的活，它直接做完汇报" | spec 不直接做。但 about_user.task_pattern (≥3 次观察) + retrospective.user_profile_distill 是 detect recurrence 的物理基础 — 已铺好 |

**关键判断**:

- **阶段一+二完全到位**:v2 spec 落地后,Mirror 功能(看见自己的工作)就能 ship
- **阶段三需要扩展但不需要重构**:user-stated intent 是 about_user 的天然延展(它本来就是 read-write 知识沉淀),加一个 kind 即可
- **阶段四+五是 agent 层**:不动数据模型,在 about_user 之上长 agent

这意味着 **v2 数据模型是 5 阶 vision 的统一基础**。但 spec 本身不实现阶段三-五;那是 alpha 反馈通过后的 v3+ 工作。

---

## 2. 目标 / 非目标

### 2.1 目标

1. **task 真正成为"完整一件事"** —— 大到"重构 attribution"、小到"订外卖" 都是平等的 task,粒度自由
2. **project 降级成可选标签** —— task 可挂 0/1/N 个 project tag,没 tag 也合法
3. **tag 系统支持多 namespace** —— project / domain / topic 等并存,扩展开放
4. **回答"今天在做什么 / 工作多久"** —— 通过查 task 的 tag 计算（挂 project tag 的 task ≈ 工作时长）
5. **时间维度叙事** —— time_segments 派生表 + LLM 起标题,支持时间线视图
6. **稳定的用户偏好层** —— about_user 表合并 Memory+Knowledge 概念,注入所有 LLM prompt
7. **多 active task 并行** —— 用户同时在做几件事是常态,attribution 不是单选

### 2.2 非目标

- **不做 event.kind 字段** —— tag system 覆盖其语义
- **不做"专注分数"** —— Rize 那种 0-100 score 是反人类设计
- **不做跨设备 about_user 同步** —— v1 全部本地
- **不做 tag namespace 内的子分类** —— v1 namespace 是扁平字符串（'project' / 'domain' / 'topic'）
- **不做 about_user 的版本管理** —— 扁平表,无分支
- **不引入新存储引擎** —— 复用 SQLite + r2d2 + jieba
- **不做 work_schedule 硬规则** —— 它是 prior,不是约束

---

## 3. 数据模型

### 3.1 `tasks` 表改造

**关键改动**：
- 删除 `project_id`(语义由 task_tags 接管)
- 删除 `active_until`(派生量,不该入表)
- `state` 收窄为三态 `{active, done, dropped}` —— `paused` 不再入库,改派生

由于走 purge-and-apply 模式,这里直接给完整 CREATE,不写 ALTER 累积:

```sql
CREATE TABLE tasks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    title           TEXT NOT NULL,
    goal            TEXT,
    summary         TEXT,
    state           TEXT NOT NULL DEFAULT 'active'
                      CHECK (state IN ('active', 'done', 'dropped')),
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_event_at   TEXT,
    last_touched_at TEXT,
    last_narrated_at TEXT,
    resolved_at     TEXT
);

CREATE INDEX idx_tasks_state ON tasks(state);
CREATE INDEX idx_tasks_last_event_at ON tasks(last_event_at);
```

**`state` 三态(终态显式 + active 默认),paused 不入库**:

| state | 含义 | 来源 |
|---|---|---|
| `active` | 默认态,新建即此 | 创建 / reopen |
| `done` | 完成,终态 | 用户显式标记 / 强信号 LLM 判定 |
| `dropped` | 放弃,终态 | 用户显式标记 |

**`effective_state` 派生(view 算,不入表)**:

```sql
CREATE VIEW tasks_with_state AS
SELECT
    t.*,
    CASE
        WHEN t.state IN ('done', 'dropped') THEN t.state
        WHEN t.last_event_at IS NULL THEN 'active'
        WHEN (strftime('%s','now') - strftime('%s', t.last_event_at)) <= 1800 THEN 'active'
        ELSE 'paused'
    END AS effective_state
FROM tasks t;
```

阈值 `1800` 秒(30 min)写在 view 里看似耦合,但它只是 UI / attribution 候选筛选时用,语义稳定。如需调整,改 view DDL(purge-and-apply 模式下零代价)。**不下沉到 config**,因为它是个语义阈值不是性能旋钮 —— 配置化只会让前后端两套阈值飘移。

**状态转移图**:

```
        ┌────────┐
        │ create │
        └───┬────┘
            ▼
     ┌──────────────┐
     │ state=active │◀────────── reopen
     └──────┬───────┘             ▲
            │                     │
            ├─ event 落入 ──┐    │
            │   (更新       │    │
            │    last_      │    │
            │    event_at)  │    │
            │               ▼    │
            │   ┌──────────────┐ │
            │   │ effective_   │ │
            │   │   state:     │ │
            │   │ active ⇄     │ │
            │   │   paused     │ │ (派生,无写库)
            │   └──────┬───────┘ │
            │          │         │
            │   显式标记终态     │
            ▼          ▼         │
       ┌──────────────────┐      │
       │ state=done /     │──────┘
       │ state=dropped    │
       └──────────────────┘
```

**关键判断**:

- `paused` 不存储 → 不需要 background timer 翻状态 → 也不需要 `task_resumption` 这个 trigger
- 新 event 落入 → 写 `last_event_at` → `effective_state` 自动从 `paused` 变 `active`,无副作用、无竞争
- 终态(done / dropped)显式标记。事后认为完成有误,显式 reopen → state 改回 active
- attribution 候选筛选条件改为 `effective_state IN ('active', 'paused')` 且 `last_event_at` 在 4h 窗口内
- 多 active 并行天然支持:任意时刻可以有 N 个 task `effective_state='active'`(订外卖 + 写代码 + 看 PR 同时 active)

### 3.2 `tags` 表（新建）

```sql
CREATE TABLE tags (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace     TEXT NOT NULL,        -- 'project' | 'domain' | 'topic' | ...
    name          TEXT NOT NULL,        -- 'Corivo' / '吃饭' / '性能优化'
    summary       TEXT,                 -- LLM 基于挂在该 tag 下的 tasks 产出
    user_renamed  BOOLEAN NOT NULL DEFAULT 0,
    status        TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'archived')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_active_at TEXT,                -- 最后一次有 task 挂入的时间

    UNIQUE (namespace, name)
);

CREATE INDEX idx_tags_namespace ON tags(namespace);
CREATE INDEX idx_tags_status ON tags(status);
CREATE INDEX idx_tags_last_active ON tags(last_active_at);
```

**namespace 取值（v1）**：

| namespace | 含义 | 例子 |
|---|---|---|
| `project` | 工作项目 | "Corivo" / "客户 A 后台" |
| `domain` | 生活领域 | "吃饭" / "健身" / "通勤" |
| `topic` | 横切主题 | "性能优化" / "安全" / "UI 设计" |

namespace 是 **开放字符串**,v1 不做 enum 约束（schema 不写死）,允许后续扩展。但 LLM prompt 里只引导产出上述三类,避免 namespace 爆炸。

### 3.3 `task_tags` 多对多关联表

```sql
CREATE TABLE task_tags (
    task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    confidence REAL,                    -- LLM 给的 0-1 分
    source     TEXT NOT NULL DEFAULT 'llm'
                 CHECK (source IN ('llm', 'user', 'auto')),
    assigned_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (task_id, tag_id)
);

CREATE INDEX idx_task_tags_task ON task_tags(task_id);
CREATE INDEX idx_task_tags_tag ON task_tags(tag_id);
```

**source 字段**：

- `llm` ：tag_assignor pass 自动判
- `user` ：用户在 UI 上手动加，**永不被自动撤销**
- `auto` ：anchor 命中规则自动加（高 confidence 的快路径）

**0 关联是合法终态**：一个 task 可以不挂任何 tag —— 这就是用户摸鱼、个人事务、探索性活动产生的 task 的归宿。UI 上把这类 task 呈现为虚拟分类 **"其他"**（见 §8.1）。它和 `project` / `domain` / `topic` 三个 namespace 平级，是合法分类桶，不是"未分类失败状态"。retrospective 的 reassign pass **不会** 反复尝试给"其他"里的 task 找 tag。

### 3.4 `tag_anchors` + `task_anchors`(替代 project_identity_anchors)

**关键改动**:不做 polymorphic FK。Tag anchor 和 task anchor 生命周期/语义都不一样,拆两张表换来真正的外键完整性 + 删除级联 + 不需要在每次查询里 dispatch `owner_kind`。

```sql
-- 删除现有 project_identity_anchors 表(数据丢弃,pre-release 项目)
DROP TABLE project_identity_anchors;

CREATE TABLE tag_anchors (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    tag_id        INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,        -- launcher/document/entity/url/path/command/name
    value         TEXT NOT NULL,        -- 锚点值,verbatim
    hit_count     INTEGER NOT NULL DEFAULT 0,
    last_hit_at   TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),

    UNIQUE (tag_id, kind, value)
);

CREATE INDEX idx_tag_anchors_tag ON tag_anchors(tag_id);
CREATE INDEX idx_tag_anchors_lookup ON tag_anchors(kind, value);

CREATE TABLE task_anchors (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id       INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    value         TEXT NOT NULL,
    hit_count     INTEGER NOT NULL DEFAULT 0,
    last_hit_at   TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),

    UNIQUE (task_id, kind, value)
);

CREATE INDEX idx_task_anchors_task ON task_anchors(task_id);
CREATE INDEX idx_task_anchors_lookup ON task_anchors(kind, value);
```

**两种 anchor 的语义区别**:

| 表 | 生命周期 | 用途 | 删除策略 |
|---|---|---|---|
| `tag_anchors` | 长期(tag 不归档就一直有效) | 长期归类。例:launcher=`ComfyUI.app` → `project:AI 生图` | tag 删除级联 |
| `task_anchors` | 短期(task 终态后转停滞) | 单 task 内的强信号。例:path=`src/services/event_attributor/mod.rs` → 这次"重构 attribution" task | task 删除级联;task done/dropped 后停止匹配(不参与候选打分) |

**两种 anchor 不冲突,各自维护**:

- 新 event 进来时,**并行**查 `tag_anchors` 和 `task_anchors`(两条独立的 `(kind, value)` 索引查询),聚合命中
- task anchor 命中权重 > tag anchor(更具体)。但仅 `effective_state IN ('active', 'paused')` 的 task 的 anchor 参与候选评分(查询时 join `tasks_with_state`)
- 同一 event 可以同时命中 task anchor + tag anchor(合理:这个 event 属于 X task,而 X task 属于 Y project tag)
- 不再需要在 application code 里 `if owner_kind == 'tag'` 分支

### 3.5 `events` 表（无变更）

**关键 1**:events 表不加 kind 字段。events 仍是无类型的"屏幕活动单元",其归属性质完全由它落入哪个 task / task 挂哪些 tag 决定。

**关键 2**:加 `attribution_state` 字段。原 draft 标"可选,不加也行" —— 那是修补思维。明确加,理由:`task_id IS NULL` 同时表达"还没处理"和"处理后判 standalone"两种语义,前者该被 retrospective 重试,后者不该 —— 不分开会导致 standalone 反复消耗 LLM 调用。

```sql
ALTER TABLE events ADD COLUMN attribution_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (attribution_state IN ('pending', 'attributed', 'standalone'));

CREATE INDEX idx_events_attribution_state ON events(attribution_state);
```

三态:

- `pending`:还没被 attributor 处理过
- `attributed`:已并入某 task(task_id 非空)
- `standalone`:attributor 处理后判定不该并入任何 task(用户摸鱼但没形成 task / 极短的孤立活动)

retrospective Pass 1 (`task_reassign`) 只扫 `attribution_state='pending'`,standalone 不进重试。

### 3.6 `time_segments` 表（draft-1 沿用）

```sql
CREATE TABLE time_segments (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at    TEXT NOT NULL,
    ended_at      TEXT NOT NULL,
    title         TEXT NOT NULL,
    summary       TEXT,
    granularity   TEXT NOT NULL DEFAULT 'semantic'
                    CHECK (granularity IN ('semantic', 'hourly', 'daily')),
    dominant_namespace TEXT,             -- 段内 task tags 多数派 namespace（替代 dominant_kind）
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),

    CHECK (started_at < ended_at)
);

CREATE INDEX idx_time_segments_range ON time_segments(started_at, ended_at);
CREATE INDEX idx_time_segments_granularity ON time_segments(granularity, started_at);
```

`dominant_namespace` 替代 draft-1 的 `dominant_kind`：段内 events.task → task_tags.tag.namespace 取多数派。仍是冗余字段,UI 渲染颜色用。

### 3.7 `about_user` 表（draft-1 沿用，小调整）

**about_user 的双重身份**：

- **作为输出**：由 retrospective Pass 5 `user_profile_distill` 从 events + user_overrides 蒸馏产出（见 §4.5）。它是观测的 **派生产出**，不是凭空的配置文件
- **作为输入**：注入到所有上游 LLM prompt 作 prior（见 §9 上下文注入策略）

这意味着 about_user 是一个 **闭环反思层** —— 来自观测，反过来指导观测。它的更新不是"用户填表"，是被持续 distill 出来的。这是 Corivo 和"普通偏好设置"的本质区别：普通偏好是 read-only 配置；**about_user 是 read-write 知识沉淀**。这个定位决定了：

- Settings UI 上要能 **溯源** —— 每条 about_user entry 显示"从哪条 event / 哪次 override 蒸馏出来的"
- 用户可以编辑/删除，但 distill 仍会持续写入新条目（永远是双向的）
- 跨设备同步时（v1 不做，v2 议题），它和 project tag 一样需要被同步 —— 因为这是"关于用户的知识"，不是设备本地状态

```sql
CREATE TABLE about_user (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    kind            TEXT NOT NULL,
    points          TEXT NOT NULL,        -- JSON array
    source          TEXT NOT NULL,        -- 'event:<id>' | 'manual' | 'override:<event_id>' | 'task_move:<task_id>'
    scope           TEXT NOT NULL DEFAULT 'global'
                      CHECK (scope = 'global' OR scope LIKE 'tag:%'),  -- ★ 改：scope 引用 tag 而非 project
    confidence      REAL NOT NULL DEFAULT 0.7,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_used_at    TEXT,
    use_count       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_about_user_kind ON about_user(kind);
CREATE INDEX idx_about_user_scope ON about_user(scope);
CREATE INDEX idx_about_user_last_used ON about_user(last_used_at);

CREATE VIRTUAL TABLE about_user_fts USING fts5(
    points,
    content='about_user',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);
```

**`kind` 取值（v1）**：

| kind | 例子 | 注入策略 |
|---|---|---|
| `tool_preference` | "倾向用 bun 而不是 npm" | 检索注入 |
| `work_habit` | "审 PR 通常先看 diff 再看评论" | 检索注入 |
| `domain_focus` | "本周在重构 attribution" | 检索注入 |
| `behavioral_constraint` | "在 Slack 团队讨论是工作,闲聊不是" | 检索注入 |
| `work_schedule` | "周一-周五 9:30-19:00" | 永远注入（条目极少） |
| `task_pattern` | "订外卖通常持续 30 分钟,涉及看-下单-验收三步" | 检索注入（**新加,task-centric 架构特有**） |

`task_pattern` 是 draft-2 新引入的：用户对"什么样的 events 应该聚成一个 task"的偏好,直接喂给 attributor 帮它判 task 边界。

### 3.8 删除的部分（破坏性）

```sql
DROP TABLE project_identity_anchors;
DROP TABLE projects;
-- projects 表完全删除。已有 project 数据可一次性迁移：每行变成 namespace='project' 的 tag
```

`projects` 表彻底删除。原有 `projects.title` → `tags(namespace='project', name=title)`。其他字段（summary / status）一一映射到 tags 对应字段。

`schema_version` 从 101 → 200（大版本跳跃,标记结构性变更）。沿用 Corivo 现有 purge-and-apply 模式。

### 3.9 不变的部分

- `screenshots` / `observations` / `facts` 三层
- `events` 表主体（除可选的 attribution_state 字段）
- `events_fts` / `tasks_fts`
- `reassignment_log`（trigger 集合扩展,见 §4）
- 整个 capture / observation / `DbInstant` 时间层

---

## 4. Pipeline 变更

### 4.0 LLM 是 escape hatch，不是默认引擎

整个 pipeline 有 9 个 LLM pass(event_boundary / task_attribute / tag_assign / tag_review / task_split / temporal_segment / user_profile_distill / task_narrate / tag_rollup)。如果每个 event 都默认走 LLM，系统会 **慢、贵、不可调试、离线不可用**。这是 V0 失败的伪学习之一(LLM 当默认)。

**v2 设计原则:LLM 是 escape hatch**。规则 + anchor 命中是默认引擎,只在 confidence 不足时把 LLM 叫起来。

具体到每个 pass 的 LLM-free fast path 触发条件:

| Pass | Fast path(无 LLM) | Escape to LLM 触发 |
|---|---|---|
| `task_attribute` | 单一候选 task 的 anchor 命中分 ≥ MERGE_THRESHOLD,且无第二候选竞争 | 多候选竞争 / 无候选但 event substantial / score 模糊 |
| `tag_assign` | 已有 tag 在该 namespace 内 anchor 命中 ≥ 2 + 无冲突 tag | 无候选 tag(需要 LLM 提议新建) / 多 tag 竞争同 namespace |
| `tag_review` | task 的 tags 命中率稳定(近 N 个 events 都命中现有 tag) | 出现 tag 命中率断崖式下降(可能漂移) |
| `task_split` | task 内连续 N 个 events 全无 anchor 重合(明显切换) | 边界模糊 / 用户疑似主动切换但 anchor 重合 |
| `temporal_segment` | 相邻 events 共 task → 同段;切换 task → 切段(boundary 已判过) | LLM 只用来 **起标题** |
| `user_profile_distill` | 见 R4 cold start bar(规则化,见 §5.0) | 越过 bar 才喂给 LLM |
| `event_boundary` | (无 fast path,boundary 是流式语义判断,必须 LLM) | 永远走 LLM |
| `task_narrate` / `tag_rollup` | (用户直接消费的产出,质量优先) | 永远走 LLM |

**实施原则**:每个 pass 的服务实现都先跑 fast path scorer,只在 escape 条件触发时再调 LLM。fast path 命中率 60% 以上(目标值)。

dogfood 后看实际命中率:
- ≥ 60%:维持
- 30-60%:调阈值 / 加规则
- < 30%:这一 pass 的 fast path 没设计对,回到 LLM-default 但留 telemetry

### 4.1 task-first attribution 流程

**旧流程**（draft-1）：

```
event → 算 project anchor 命中 → 选 task → 落库
```

**新流程**（task-first）：

```
event → 候选 active tasks（state in {active, paused}, 且时间窗口合理）
      → 算每个候选 task 的匹配分（task anchor + 内容相似度）
      → 命中阈值: 并入 / 否则开新 task（task_id=NULL → tag_assignor 后续打 tag）
      → standalone 兜底: 完全无关的孤立 event → attribution_state='standalone'
```

**关键变化**：

- attribution 的核心问题变成 **"event 该并入哪个 active task / 还是开新"**,不是 "event 该挂哪个 project"
- project tag 是 **task 形成后** 由 tag_assignor 打的,**不在 attribution 主路径**
- 多 active 并行时,attribution 在 N 个候选中选,而不是单选 project

### 4.2 services/event_attributor → services/task_attributor 改名 + 重写

**入口逻辑**:

```rust
// pseudo-code
fn attribute(event: &Event) -> AttributionResult {
    // 候选 = effective_state ∈ {active, paused} 且 last_event_at 在 4h 窗口内
    let candidates = list_attribution_candidates(window = 4h);
    let scored = candidates.iter().map(|t| (t, score(event, t))).collect();
    let best = scored.max_by_score();

    match best {
        Some((task, score)) if score >= MERGE_THRESHOLD => {
            attach_event_to_task(event, task);   // 写 last_event_at,effective_state 自动算
            AttributionResult::Merged(task.id)
        }
        _ => {
            // 评估是否值得开新 task
            if is_event_substantial(event) {
                let new_task = create_task_from_event(event);
                AttributionResult::NewTask(new_task.id)
            } else {
                mark_event_standalone(event);
                AttributionResult::Standalone
            }
        }
    }
}
```

注意:**没有显式的 "Resume" 分支**。因为 `paused` 是派生量 —— 一旦 event 落入 paused task,`last_event_at` 更新,view 算出来的 `effective_state` 自动从 paused 变 active,无需任何状态翻转代码。这就是 §3.1 删 `active_until` 的直接收益。

**score 函数构成**:

- task_anchors 命中(强信号,占比 50%)
- tag_anchors 命中(task 已挂的 tags 的 anchor,中信号,30%)
- 内容相似度(LLM-free 的 BM25 / token overlap,弱信号,20%)

**when score 不确定时启用 LLM**:用 `task_attribute.md` 让 LLM 在 top-3 候选中选。

### 4.3 services/tag_assignor（新服务）

**职责**：task 形成或更新后,给它打 tags（多个 namespace）。

**触发时机**：

- task 新建时（同步,在 attributor 之后立即跑）
- task 累积 ≥ 5 个新 events 时（异步,batch）
- retrospective 跑 split 后产生新 task 时

**流程**：

```
1. 收集 task 内所有 events 的 facts + 已挂 tags 的 anchor 命中
2. 候选 tags：
   - 现有 tags 中按 anchor 命中排序的 top-N
   - LLM 提议的新 tags（如果现有 tags 都不合适）
3. LLM 决策（prompt: tag_assign.md）：
   - 对每个候选 tag,return assign / skip
   - 对每个 namespace 最多挂 1 个 tag（不强制,但 prompt 里引导）
   - 可以判定 "all skip"（task 不挂任何 tag,合法终态）
4. 写库 task_tags + 必要时新建 tags 行
5. 命中的 anchor 调 hit_count += 1
```

**关键约束**：

- tag_assignor **不写 anchor**（anchor 由独立的 anchor_extractor pass 写,见 §4.4）
- "all skip" 是合法的（个人事务、摸鱼活动可能就不该有任何 tag）
- user_source='user' 的 task_tags **永不被自动覆盖**

### 4.4 services/anchor_extractor

**职责**:从 task 的 events.facts 抽出可能的 anchor,分别写入 `tag_anchors` 和 `task_anchors`。

**两条线**(独立实现,但合并进 tag_assignor 的同一次 LLM pass —— prompt 同时输出 tag 决策 + anchor 候选):

- **tag_anchors 写入**:tag 第一次被分配时,从触发该分配的 task 抽 3-5 个高频 fact 当初始 anchor;后续每次 `tag_anchors.hit_count++`,长期 hit_count 高的同源 fact 自动追加为新 anchor
- **task_anchors 写入**:task 内 events 的 facts 中反复出现的 path / entity 直接落 `task_anchors`,在 `effective_state IN ('active', 'paused')` 期间参与匹配;task 终态(done / dropped)后,task_anchors 不主动转移到 tag_anchors —— 让 tag_anchors 的写入路径只有一个(via tag_assignor),避免双源更新冲突。停滞的 task_anchors 行物理保留(用于历史溯源 / 用户 reopen),但不参与候选评分

**为什么不在 task done 时把 task_anchors 升级到 tag_anchors**:升级逻辑要回答"哪些 task_anchor 值得提升"——这本质是 tag_assignor 已经在做的事(它在 task 形成时就决定了哪些 anchor 进 tag_anchors)。再加一条升级路径只会让 tag_anchors 变成"两个上游、状态难推"。

### 4.5 retrospective_reviewer：5 个 pass

| Pass | 频率 | 作用 |
|---|---|---|
| 1. `task_reassign` | 5 min | task_id IS NULL（且 attribution_state='pending'）的 events 重新尝试归属 |
| 2. `task_split` | 5 min | 拆分漂移的 task（沿用现有逻辑） |
| 3. `tag_review` | 15 min | 重新评估近期 task 的 tags（merge 重复 tags / 修错的 tag） |
| 4. `temporal_segment` | 15 min | 把最近 events 切成 time_segments |
| 5. `user_profile_distill` | 30 min（debounced） | 把 override 信号沉淀到 about_user |

**Pass 3 `tag_review` 的存在意义**：tag_assignor 是单次 pass,但 task 持续累积 events 后,初始 tag 可能不准（"看起来是 Corivo 项目,后来发现是客户 A 项目"）。tag_review 用更长的 task 历史重判。

**reassignment_log trigger 扩展**：

| trigger | 来源 |
|---|---|
| `user_drag` | 用户手动移动 event |
| `user_split` | 用户手动拆分 task |
| `user_tag_change` | 用户手动改 task 的 tag |
| `attribution_initial` | event_attributor 首次归属 |
| `task_reassign_auto` | retrospective pass 1 |
| `task_split_auto` | retrospective pass 2 |
| `tag_review_auto` | retrospective pass 3 |

(原 draft 的 `task_resumption` trigger 不存在了 —— paused 是派生态,新 event 落入直接更新 `last_event_at`,无需独立 trigger。)

### 4.6 idle gap 推断（不入表,UI 渲染时算）

沿用 draft-1：

| gap 长度 | 类型 | UI 渲染 |
|---|---|---|
| < 2 min | 忽略 | 紧邻直接相连 |
| 2-15 min | break | 灰色细条 |
| 15-60 min | off（短） | 空白 |
| ≥ 60 min 或跨 logical day | off（长） | 断开 |

logical day 切点 v1 硬编码 05:00。

---

## 5. Prompt 设计

> 哲学来源:OpenChronicle 的多层管线 + 强职责隔离 + verbatim 贯穿。
> 本章把跨 prompt 共用的通则、改造点、完整新 prompt 骨架收在一起,跟数据模型同章定义,避免规则散落各处。

### 5.0 七条通则

跨 prompt 复用的规则,先列在前,后面具体 prompt 直接引用 R1~R7。

#### R1 · Verbatim preservation rule

> Authored text, URLs, window titles, file paths, command invocations, and proper
> nouns MUST appear verbatim in `facts` values. Anchor matching depends on this
> fidelity. Paraphrasing breaks attribution.

适用:`event_boundary.md` 的 `facts[]` 提取阶段、`tag_assign.md` 的 `initial_anchors`。

#### R2 · Anti cross-attribution rule

> A single observation window may contain several independent surfaces (multiple
> browser tabs, multiple chat conversations, multiple files). NEVER cross-multiply:
> a person seen in conversation A and a topic seen in conversation B must NOT be
> combined into "discussed B with [person from A]". If a fact only appeared next
> to App X, do not associate it with App Y.

适用:所有 prompt(event_boundary / task_attribute / tag_assign / retrospective_reassign)。

#### R3 · 层间通信协议(structured signal fields)

上游 prompt 在自己的输出里留**结构化标记**给下游使用,下游信任标记、降低自己的判断 bar、节省 LLM 调用。

| 标记字段 | 产出方 | 消费方 | 含义 |
|---|---|---|---|
| `regularity_signal` | event_boundary | user_profile_distill | "这个 event 直接观察到一个可记录的偏好/规律" |
| `strong_anchor_signal` | event_boundary | task_attributor | "anchor 信号特别强,可跳过 LLM 二次判断" |
| `clean_split_point` | retrospective_split_task | task_attribute | "这是干净的 split 边界,下次不用再判" |

每个标记都是 `string | null`,**默认 null**。规则:

- 标记不是必填,>80% 的 event 应该是 null
- 一旦填了,下游必须信任(不再独立验证)
- 标记内容是简短证据句,不是结论 ── "user typed: 我用 bun 不用 npm" 是好标记,"user prefers bun" 不是

(原 draft 的 `tag_unstable` 字段被删除 ── tag_assign 的 default 已经是"skip 不确定的候选",输出空数组就是它的"我没想清楚"信号,无需再加一个独立字段。)

#### R4 · Cold start 分层 bar

不同类型的事实,第一次能否写入的 bar 不同(实施位置:`user_profile_distill.md` 的判断逻辑):

| about_user.kind | 写入 bar |
|---|---|
| `work_schedule` | 用户明确说 → 1 次写入 |
| `behavioral_constraint` | 用户明确说 → 1 次写入 |
| `tool_preference` | 用户明确说 OR 上游 `regularity_signal` → 1 次写入 |
| `work_habit` | ≥ 2 次独立观察 → 写入;单次 → skip |
| `domain_focus` | ≥ 2 次独立观察 |
| `task_pattern` | ≥ 3 次独立观察(task 形成模式更难判,bar 更高) |

#### R5 · Default action: write nothing / stay current

每个会"写库"或"做归属"的 prompt,**首段就放默认行为声明**:

- `task_attribute.md`:default 倾向 `merge_into` 已存 task,不要轻易开新
- `tag_assign.md`:default skip 不确定的候选,empty actions 是合法的
- `tag_review.md`:default 保持现状(empty actions)
- `user_profile_distill.md`:default 写空数组,>70% 应 skip

#### R6 · Authorship guard

输入框打字 ≠ 参与对话。在 chat / IM 类应用里,"在搜索框打字"应描述为搜索/导航行为:

```
Treat typing in a message composer as authoring/participation. However, if
the focused editable input is clearly a search box / address bar (title
contains "search" / "find" / "url" / "address" / "omnibox" / "command"),
describe it as searching/navigating.
```

适用:`event_boundary.md` 的 fact 抽取与 event_summary 描述。draft-2 不再用 authorship guard 判 kind(kind 不存在了),只用于描述准确性。

#### R7 · Pattern confirmation via tool(v2)

判断"这是新偏好还是旧偏好的重复"时,不是塞更多 input,而是给 LLM 工具:

```
search_about_user(query, top_k=5)
search_recent_facts(query, days=30)
search_tasks_by_anchor(kind, value, days=30)
```

让 LLM 主动调,命中 ≥ 2 次才升级为 pattern。比"硬塞历史 input"便宜,产出更稳。适用:`user_profile_distill.md`、`task_attribute.md`(v2 升级)。v1 用 input 里塞 top-K 替代。详见 §5.5 工具调用升级路线。

---

### 5.1 删除 / 废弃

- `event_boundary.md` 中所有 kind 相关字段和 hard rules
- `kind_review.md` 不需要了

### 5.2 改造现有 prompt

#### 5.2.1 `event_boundary.md`(中度改造)

**Patch A** ── 顶部加 "Time + Adjacency" Layer 1 上下文,在 `## Inputs` 之前插入新章节:

```markdown
## Context

### Time
- Now: {{now_local}} ({{weekday_local}})
- User work schedule: {{work_schedule_or_unset}}
- Within work hours? {{yes|no|unset}}

### Adjacent Event
{{prev_event_or_none}}
  Format: task="...", tags=[...], summary="..."

### Active Tasks (top 3 by recency + anchor)
{{active_tasks_brief}}
```

**Patch B** ── 删除 `## Kind decision` 整段。boundary 决策只关心 continue/end_and_start/background。

**Patch C** ── 输出 schema 简化:

```json
{
  "decision": "continue | end_and_start | background",
  "event_title": "...",
  "event_summary": "...",
  "is_background": false,
  "confidence": 0.0,
  "reasoning": "...",

  "regularity_signal": null,
  "strong_anchor_signal": null,

  "facts": [...]
}
```

**Patch D** ── 保留 R1 Verbatim、R2 Anti-cross-attribution、R6 Authorship guard 三段。

#### 5.2.2 `event_task_attribute.md` → `task_attribute.md` 改名 + 重写

完整骨架见 §5.3.1。原 prompt 选 project 下的 task,新 prompt 选并入哪个 active task / 开新 / standalone。

#### 5.2.3 `retrospective_reassign.md`(小改)

- candidate 列表语义改变:旧候选 = projects + tasks;新候选 = active tasks(不再有 projects)
- 保留 R2 anti-cross-attribution 段
- 保留 R1 verbatim rule for `initial_anchors`(写入 `task_anchors` 表)

#### 5.2.4 `retrospective_split_task.md`(极小改)

输出 schema 加 `clean_split_point` 字段(R3 层间通信协议)。其余沿用。

#### 5.2.5 `task_narrate.md` / `tag_rollup.md`

- `task_narrate.md`:不动(task 仍是被 narrate 的对象,project 改 tag 不影响其结构)
- `tag_rollup.md`:取代 `project_rollup.md`,逻辑相似("对挂在某 tag 下的所有 active+recent tasks 做滚动总结")。允许产出 `Observed regularity:` 行(参考 OpenChronicle narrator)

### 5.3 新建 prompt 完整骨架

#### 5.3.1 `task_attribute.md`

```markdown
# Task attribution (task-first)

You decide whether a new event should merge into one of several currently
active tasks, or whether to start a new task.

## Default action (R5)

**Prefer merging into an existing active task** when reasonable. New tasks
fragment the user's narrative — only create one when the event genuinely
represents a new "thing" the user is doing.

## Inputs

- `event` — the new event (title, summary, facts, started_at)
- `active_tasks[]` — top-N tasks where effective_state ∈ {active, paused},
  ranked by anchor + recency, each with title, summary, last_event_at, sample
  recent events, current task_anchors
- `relevant_user_preferences[]` — about_user top-K (focus on `task_pattern` kind)

## Three actions

- `merge_into` — provide `task_id` of an active/paused task. Use when:
  - event's facts show strong overlap with task_anchors of a candidate
  - subject/topic continuity is clear
  - last_event_at is recent (< 30 min default, can stretch to 2h with strong signal)
- `new_task` — provide `proposed_title` (concise, 2-8 words). Use when:
  - the event represents a genuinely new "thing" (订外卖, 开始审 PR-XXX, 重构 Y)
  - no active task is a reasonable fit
- `standalone` — neither merge nor new task. Use when:
  - event is too short / fragmentary / context-switch noise
  - explicit non-work browsing without sustained engagement

## Hard rules

- NEVER merge into a task whose namespace tags clearly contradict the event's
  facts (R2). E.g., event is about "订外卖" but candidate task has tag `project:Corivo`.
- NEVER create a new task with a title that's a generic verb ("Working",
  "Browsing"). If you can't propose a concrete name, choose `standalone` instead.
- If `event.strong_anchor_signal` is non-null (R3), lean strongly toward
  `merge_into` the task whose anchor matched.

## Output

```json
{
  "action": "merge_into" | "new_task" | "standalone",
  "task_id": 42,                    // only if merge_into
  "proposed_title": "...",           // only if new_task
  "confidence": 0.0,
  "reasoning": "..."
}
```

Return only JSON.
```

#### 5.3.2 `tag_assign.md`

```markdown
# Tag assignment

You decide which tags to attach to a task. A task can have 0 to N tags.
Skipping all tags is a valid outcome for personal/leisure tasks.

## Default action (R5)

**Skip uncertain candidates.** A wrong tag pollutes downstream filtering and
narration. The cost of missing a tag is far smaller than wrongly tagging.

## Inputs

- `task` — the task being tagged (title, summary, recent events, current tags if any)
- `candidate_tags[]` — existing tags ranked by anchor hit count + namespace
  diversity, each with namespace, name, summary, hit_evidence
- `relevant_user_preferences[]` — about_user filtered to tag-relevant kinds
- `available_namespaces` — { 'project', 'domain', 'topic' } (v1)

## Per-tag decision

For each candidate, choose:

- `assign` — clear evidence the task belongs under this tag
- `skip` — weak evidence / ambiguous / would only fit one of many events

## New tag proposal

Only when NO existing tag in a needed namespace fits, propose a new tag:

```json
{
  "action": "new_tag",
  "namespace": "project | domain | topic",
  "name": "...",
  "initial_anchors": [
    { "kind": "launcher", "value": "verbatim from facts" }
  ]
}
```

`initial_anchors` MUST be copied verbatim from the task's events' facts (R1
verbatim rule).

## Hard rules

- Per namespace, prefer at most ONE tag (a task is "primarily" one project,
  one domain, etc.). Multiple tags within the same namespace are allowed but
  should be rare (e.g., a refactor task that touches two projects).
- NEVER assign `project` namespace tag to a task whose events show no work
  artifact (no editor, no code, no PR/doc). Such tasks are likely
  domain/personal — leave `project` empty.
- `domain` namespace is appropriate for personal-life tasks (eating,
  commuting, family). Do not invent generic `domain:work` — that's what
  `project` namespace is for.

## Output

```json
{
  "actions": [
    { "action": "assign", "tag_id": 7, "confidence": 0.0, "reasoning": "..." },
    { "action": "new_tag", "namespace": "project", "name": "...",
      "initial_anchors": [...], "confidence": 0.0, "reasoning": "..." }
  ]
}
```

Empty actions array is valid (task gets no tags). Return only JSON.
```

#### 5.3.3 `tag_review.md`

retrospective Pass 3 用。给 task 累积更多 events 后,重判 tag 是否还合适。结构同 `tag_assign.md`,但 default action 改为**保持现状**:

```markdown
## Default action (R5)

**Return empty actions array (keep current tags).** Tag stability is more
valuable than minor accuracy gains. Only propose changes when:

- a current tag is clearly wrong (events have shifted topic completely)
- a needed namespace is empty AND now has clear evidence
- two tags in the same namespace clearly overlap (propose merge)
```

#### 5.3.4 `temporal_segment.md`

```markdown
# Temporal segmentation

You are grouping a person's day into 5–15 narrative "segments" for a
timeline view.

Each segment is a consecutive run of events that belong together thematically.

## Inputs

- `events` — ordered list (1-based) with id, started_at, ended_at, title,
  summary, task_id, task_tags (namespace + name list). Already sorted by
  started_at.

## Rules

- Respond with STRICT JSON:
  `{"segments": [{"start_idx": 1, "end_idx": 3, "title": "...", "subtitle": "..."}]}`
- start_idx / end_idx are 1-based inclusive indexes
- Segments must NOT overlap
- Every event does NOT need to be covered — OK to skip 1–2 fragmentary events
- Aim for 5–15 segments. Err toward FEWER, LARGER segments
- Title: 2–8 words. Subtitle: optional, 4–12 words
- Match the day's language (中文 if event titles are mostly 中文)
- Use stable codenames from event titles verbatim (R1)
- NEVER cross-multiply names from non-adjacent events into one segment title (R2)

## Default (R5)

If a stretch is genuinely fragmentary (4+ rapid context switches), do NOT
force a segment. Skipping is better than mis-narrating.

Return only JSON.
```

`dominant_namespace`(time_segments 表)在调用方用 task_tags 多数派算,不在 prompt 内输出。

#### 5.3.5 `user_profile_distill.md`

```markdown
# User profile distillation

You extract durable user preferences from recent override signals and
high-frequency behaviors, then decide whether to write them into about_user.

## Default action (R5)

**Write nothing.** Most distill runs (>70%) should return empty actions array.
Bad preferences poison every downstream prompt.

## Inputs

- `recent_overrides` — events/tasks where the user manually changed task
  assignment or tag, with original LLM judgment + user correction
- `recent_regularity_signals` — events with non-null regularity_signal (R3)
- `existing_about_user` — top-K existing entries with [id=X] tags

## Three-action model per candidate

- `create` — genuinely new
- `update` — refinement of existing entry (provide update_id)
- `skip` — duplicate / weak / one-off / inferred without grounding

**Prefer update over create.**

## Cold start bar (R4)

| kind | bar |
|---|---|
| `work_schedule` | explicit user statement → 1 |
| `behavioral_constraint` | explicit user statement → 1 |
| `tool_preference` | explicit OR upstream regularity_signal → 1 |
| `work_habit` | ≥ 2 independent observations |
| `domain_focus` | ≥ 2 independent observations |
| `task_pattern` | ≥ 3 independent observations |

## Anti-hallucination

- Every point must be grounded in input text. Quote evidence in reasoning.
- NEVER invent project/person/tool names not in inputs.
- NEVER restate "user did X" as "user prefers X" without explicit statement
  or repeated observation.

## Output

```json
{
  "actions": [
    {
      "action": "create | update | skip",
      "update_id": 42,
      "kind": "tool_preference",
      "points": ["..."],
      "scope": "global | tag:7",
      "confidence": 0.0,
      "reasoning": "evidence sentence"
    }
  ]
}
```

`scope` 支持 `global` 或 `tag:<id>`(replaces draft-1 的 `project:<id>`)。
新增 kind `task_pattern` 是 task-centric 架构特有,作为重点关注。
Use the user's actual name in `points`. Empty actions is the expected default.
Return only JSON.
```

### 5.4 落地顺序(与 §10 上线节奏对齐)

| §10 Step | Prompt 改动 |
|---|---|
| Step 1 | 仅 schema,无 prompt 改 |
| Step 2 | §5.3.1 task_attribute.md 新建 + §5.2.1 event_boundary.md 改造(删 kind) |
| Step 3 | §5.3.2 tag_assign.md 新建 |
| Step 4 | §5.3.3 tag_review.md + §5.3.4 temporal_segment.md + §5.2.3 retrospective_reassign 改造 |
| Step 5 | 各 prompt 注入 about_user(改而非新建) |
| Step 6 | §5.3.5 user_profile_distill.md 新建 |

### 5.5 工具调用升级路线(v2)

v1 走 "input 塞 top-K" 简化路径。v2 引入工具调用:

```
user_profile_distill 工具集:
  search_about_user(query, top_k)
  search_recent_facts(query, days)
  read_about_user(id)

task_attribute 工具集:
  search_active_tasks(query, top_k)
  read_task(id)
  search_anchors_by_value(kind, value)
```

升级触发:
- v1 跑稳后,distill 重复写入率 > 10%(说明 input 塞 top-K 不够)
- 或 attribution 在多 active task 场景下出错率 > 阈值

### 5.6 验证 prompt 改造的方法

每个 patch 上线前后,跑同组采样对比:

1. **同样 input,旧 prompt vs 新 prompt 各跑一次**,diff 输出
2. **重点采样**:低 confidence 的 events、含中英文混排的、跨多 app 的、典型 personal/leisure 场景的
3. **人工核对 50 条**:verbatim 是否保留?cross-attribution 是否消除?task 边界是否合理?tag 是否准确?regularity_signal 是否准确?
4. **fact 质量审计**:抽样 100 个新 facts,看 anchor 命中率上升

如果新 prompt 在某维度明显劣化,回滚单独修。不要"一次改全"。

---

## 6. 服务结构

```
services/
├── observation_session/        # 不动
├── task_attributor/            # 原 event_attributor 改名 + 重写
│   ├── mod.rs                  # task-first 主流程
│   ├── scoring.rs              # task anchor + tag anchor + 内容相似度打分
│   └── prompts.rs              # task_attribute.md
├── tag_assignor/               # 新增
│   ├── mod.rs                  # task 形成后打 tag
│   ├── anchor_extractor.rs     # 从 task events 抽 anchor（双 owner）
│   └── prompts.rs              # tag_assign.md
├── retrospective_reviewer/     # 5 pass
│   ├── mod.rs
│   ├── pass_task_reassign.rs   # 现有 reassign 改造
│   ├── pass_task_split.rs      # 现有 split 沿用
│   ├── pass_tag_review.rs      # 新增
│   ├── pass_temporal_segment.rs # 新增
│   └── pass_user_profile.rs    # 新增
├── about_user/                 # 新增
│   ├── mod.rs                  # CRUD
│   ├── retriever.rs            # FTS5 + kind 过滤
│   └── prompts.rs              # user_profile_distill.md
├── task_narrator/              # 现有,基本不动
├── tag_narrator/               # 新增（替代 project_rollup）
└── ...
```

---

## 7. Tauri Commands

新增 / 改造的 commands（在 `commands/workspace.rs`）：

```rust
// === Task ===
// 默认返回 effective_state ∈ {active, paused};window_hours 过滤 last_event_at
list_active_tasks(window_hours: u32) -> Vec<TaskWithState>
get_task(id: i64) -> TaskWithState
move_event_to_task(event_id: i64, task_id: Option<i64>) -> ()
split_task_at(task_id: i64, split_from_event_id: i64, new_title: String) -> TaskWithState
merge_tasks(source_id: i64, target_id: i64) -> ()
mark_task_done(task_id: i64) -> ()
mark_task_dropped(task_id: i64) -> ()
reopen_task(task_id: i64) -> ()              // state: done|dropped → active

// === Tags ===
list_tags(namespace: Option<String>) -> Vec<Tag>
get_tag(id: i64) -> Tag
add_tag_to_task(task_id: i64, tag_id: i64) -> ()       // source='user'
remove_tag_from_task(task_id: i64, tag_id: i64) -> ()
create_tag_manual(namespace: String, name: String) -> Tag
rename_tag(tag_id: i64, new_name: String) -> ()
archive_tag(tag_id: i64) -> ()

// === Time-based queries ===
get_namespace_breakdown(date: String) 
    -> { project_secs, domain_secs, topic_secs, untagged_secs, online_secs }
list_time_segments(date: String, granularity: Option<String>) -> Vec<TimeSegment>

// === About-User ===
list_about_user(scope: Option<String>) -> Vec<AboutUserEntry>
upsert_about_user_manual(entry: AboutUserUpsert) -> i64
delete_about_user(id: i64) -> ()
```

对应 `src/lib/tauri.ts` wrapper + `src/hooks/use-workspace.ts` React Query hooks。

---

## 8. Frontend 变更

### 8.1 `/now` 统一视图（view switch）

**单页面、双视图**。`/timeline` 不做独立路由 —— 项目维度和时间维度是 **同一份数据的两种投影**，UI 上用 toggle 切换更对得上数据本质，且能保持上下文连续：

```
┌─ /now ──────────────────────────────────────────────┐
│  视图: ● Project   ○ Timeline                       │
│  ──────────────────────────────────────────────     │
│  [视图根据 toggle 渲染]                              │
└──────────────────────────────────────────────────────┘
```

切换时保留选中状态：

- **Project → Timeline**：保留当前选中的 task，自动滚动 timeline 到该 task 第一个 event 的时间
- **Timeline → Project**：保留当前选中的 segment，Project 视图高亮该 segment 涉及的 tasks

#### 8.1.1 Project 视图（默认）

三栏：`Tag 列表（按 namespace 分组）→ Task 列表 → Event 时间线`

左栏的 tag 列表默认折叠为：

```
▼ Project (5)
  - Corivo (active, 12 tasks)
  - 客户 A 后台 (paused)
  ...
▼ Domain (3)
  - 吃饭
  - 通勤
  - 健身
▼ Topic (2)
  - 性能优化
  - 安全
▷ 其他 (3)        ← 没挂任何 tag 的 task，合法终态而非"未分类失败"
```

点击某个 tag → 中栏显示挂该 tag 的 tasks（active + recent paused/done）。**"其他" 桶**里是未挂任何 tag 的 tasks —— 个人事务、摸鱼、探索性活动等都在这里。它和上面三个 namespace 平级，不是异常状态。

#### 8.1.2 Timeline 视图

```
顶部:  日期选择器 (今天 ◀ ▶)
左侧:  time_segments 列表
       [10:00-11:30] PR review 深潜
        ↳ 主要 tag: project:Corivo, dominant=project
        ↳ 5 个 events，3 个 tasks
右侧:  选中段后展开看 events，事件卡片显示其归属 task + tags
```

### 8.2 时长统计 widget

`/now` 顶部紧凑卡片,基于 `get_namespace_breakdown`：

```
┌─ 今日时长 ─────────────────────────────────────┐
│ ████████████░░░░░░ project 4h32m              │
│ █████░░░░░░░░░░░░░ domain  1h18m              │
│ ██░░░░░░░░░░░░░░░░ topic   0h28m              │
│ ███░░░░░░░░░░░░░░░ untagged 1h05m              │
│ 在线 7h22m · 离线 4h08m                       │
└──────────────────────────────────────────────────┘
```

注意：用 namespace 而非 kind 着色。"工作时长" = project namespace 时长。

### 8.3 Event card 操作菜单

右键菜单：

```
移动到任务...
  ▸ 当前 active 的 N 个 tasks
  ▸ 创建新任务...
  ▸ 标记为 standalone (不归属任何任务)
```

不再有"改 kind"菜单（kind 不存在了）。

### 8.4 Task card 操作菜单

```
标签...
  ▸ 添加 tag (namespace 选择 + 现有/新建)
  ▸ 移除 tag
任务...
  ▸ 标记完成
  ▸ 拆分任务（在某 event 之后）
  ▸ 合并到其他任务...
```

### 8.5 Settings

新增标签页：

- **About-User**：列表 + 手动新增 + 编辑 / 归档 + **溯源链**（每条显示从哪条 event / 哪次 override 蒸馏出来的）
- **Work Schedule**：周一-周日七天工作时段（写入 about_user `kind=work_schedule`）
- **Tags**：管理所有 tags（重命名、归档、合并、查看 anchor 列表）

---

## 9. 上下文注入策略（about_user）

### 9.1 检索

每次需要注入 about_user 时：

1. FTS5 查询用 event/task summary 当 query
2. 限制 `kind != 'work_schedule'`（schedule 单独永远注入）
3. BM25 排序取 top 5
4. scope 过滤：`global` 或匹配当前 task 已挂的 tags
5. 取 final top 3 注入,调 `mark_used` 更新统计

### 9.2 注入位置

| Prompt | about_user 注入策略 |
|---|---|
| `event_boundary.md` | top 3 + work_schedule（每次） |
| `task_attribute.md` | top 3 偏 `task_pattern` kind |
| `tag_assign.md` | top 3 偏 `behavioral_constraint` kind |
| `retrospective_reassign.md` | top 3 |
| `temporal_segment.md` | 不注入（中性的时间叙事任务） |
| `user_profile_distill.md` | 注入 existing_about_user top-K（用于 update over create） |

### 9.3 Token 预算

| Prompt | about_user 占用 | 总预算 |
|---|---|---|
| event_boundary | ~300 tokens | 不超过 1500 总 prompt |
| task_attribute | ~200 tokens | ≤ 2000 |
| tag_assign | ~200 tokens | ≤ 1500 |

---

## 10. 上线节奏

**严格按顺序,每一步独立可验证。每一步上线后跑 1-3 天看实际产出再进下一步。**

### Step 1 · Schema 大重构（破坏性）

- schema_version 101 → 200
- 删除 projects + project_identity_anchors 表
- 新建 tags / task_tags / tag_anchors / task_anchors / time_segments / about_user / about_user_fts
- 新建 view tasks_with_state(派生 effective_state)
- 改造 tasks(删 project_id,删 active_until,state CHECK 收窄为 {active, done, dropped},加 last_event_at)
- 改造 events:加 `attribution_state` ENUM {pending, attributed, standalone} ── 必加,不再标"可选"。理由:retrospective Pass 1 需要明确知道哪些 event 该重试,`task_id IS NULL` 不区分"还没跑过"和"跑过判 standalone",前者要重试后者不该
- **不动现有任何代码,先让 schema 跑通**
- 验证:本地起 app,DB 文件被正确清空重建

### Step 2 · task_attributor task-first 流程

- 改写 event_attributor → task_attributor
- prompt: `task_attribute.md` 上线
- 此时 task 不挂任何 tag（tag_assignor 还没建）
- /now 中栏看 tasks 列表,左栏空（没 tag）
- 验证：dogfood 一两天,看 task 边界判断准不准（多 active 并行场景）

### Step 3 · tag_assignor + 左栏 tag 视图

- 新服务 tag_assignor + anchor_extractor
- prompt: `tag_assign.md`
- /now 左栏出现 tag 树
- /now 顶部时长 widget 上线（namespace 维度）
- 验证：tag 自动分配是否合理,空 tag 的 task 是否真的是 personal/leisure

### Step 4 · retrospective passes

- pass 1 task_reassign（改造现有）
- pass 2 task_split（沿用）
- pass 3 tag_review（新）
- pass 4 temporal_segment（新）
- `/now` 加 view switch toggle，Timeline 子视图上线
- 验证：tag 漂移修正、time_segment 切分质量、Project↔Timeline 切换上下文保留

### Step 5 · about_user 表 + Settings UI（手动写入）

- 表 + retriever
- Settings 加 About-User 标签页 + Work Schedule 表单 + Tags 管理
- 各 prompt 顶部注入 about_user
- **暂不做 distill（pass 5 不上）**
- 验证：手动维护偏好能否提升 attribution / tag 准确率

### Step 6 · user_profile_distill pass

- pass 5 上线
- 用户 override（task 移动 / tag 改动 / event 重归属）→ debounced distill → about_user 自动写入
- 验证：override 后下次类似场景是否自动判对

### 不做的事

- v1 不做 about_user 跨设备同步
- v1 不做 namespace 子分类
- v1 不做 vector embedding 检索
- v1 不做 productivity score
- v1 不做 break/off 自动暂停 task

---

## 11. 已定 default + Open Questions

### 11.1 已定 default(写死,无需 dogfood 才决)

这些原本在 draft 里挂"凭直觉,dogfood 验证"的悬空项,现统一锁定 default。要调整就动数值,不再"等数据再决定要不要做"。

| 议题 | Default | 实施位置 | 调整门槛 |
|---|---|---|---|
| active 候选窗口 | 4h(`last_event_at` 在 4h 内的 task 才进 attribution 候选) | `task_attributor::list_attribution_candidates` | 改常量 |
| active 派生阈值 | 30 min(`tasks_with_state` view 里写死 1800 秒) | `db/schema.sql` view DDL | purge-and-apply 改 view |
| task 自动转 done | **永不自动**。done/dropped 都是显式标记(用户 / LLM 强信号判定),无 N 天超时 | `tasks.state` 终态规则 | 加新 retrospective pass |
| anchor 写入策略 | 增量(命中 hit_count++,新 fact 触发新 anchor;无定期全量重算 pass) | `tag_assignor` + `anchor_extractor` | 见 §11.2 Q3 |
| logical day 切点 | 05:00 硬编码(本地时区) | `db::time::logical_day_of` | v2 从 work_schedule 推断 |
| break / off 阈值 | < 2 min 忽略 / 2-15 min break / 15-60 min off-short / ≥ 60 min off-long | UI 渲染层 idle gap | 改常量 |
| time_segments 跨夜 | 按 logical day(05:00)划,跨段拆两段 | `pass_temporal_segment` | 不调 |
| schema 版本 | 101 → 200(大版本跳跃,标记结构性变更) | `db/migrations.rs` | 不调 |
| 每 namespace tag 数 | 软约束 ≤ 1(prompt 引导,DB 不限制)。允许例外(refactor 跨两个 project) | `tag_assign.md` Hard rules | 见 §11.2 Q1 |

### 11.2 真正的 Open Questions(需要数据回答,无法靠拍脑袋定)

剩下四条留作真问题,各有明确触发条件 / 验收方法,不再是"先写出来,以后再说":

**Q1 · namespace 多 tag 是否升级硬约束?**
当前软约束。dogfood 1 个月后看 `task_tags` 表里"同 task 同 namespace 多 tag"的比例:
- < 5%:维持软约束
- 5-15%:加 prompt 例外白名单
- \> 15%:DB 加 trigger 强制 unique(task_id, namespace) ── 但这意味着 schema 多一层约束,慎重

**Q2 · tag 命名 canonical 怎么保证?**
现在靠 `unique(namespace, name)` 防重 + prompt 引导。真问题在重命名时:用户把 `project:Corivo` 重命名为 `project:corivo-app`,所有历史 task_tags 自动迁移没问题,但 tag_anchors 的 hit_count 历史是不是也该一并保留?(现在 schema 是 ON DELETE CASCADE,rename 不删,所以 OK ── 但如果用户拆分一个 tag 成两个,anchor 怎么分?这才是真问题。)等真有用户拆分 tag 的需求再决定。

**Q3 · anchor 漂移检测的指标是什么?**
增量策略下,旧 anchor 可能持续命中无关 event(典型:用户把 ComfyUI.app 既用于 AI 生图也用于做 demo)。需要一个"anchor 命中分布"指标 ── 比如某 anchor 30 天内命中的 task,挂的 tag 是否分散?分散度 > 阈值 → 触发人工复核 / LLM 重判。但具体阈值要看真实数据分布。**dogfood 1 月后再定**。

**Q4 · task 粒度爆炸的 UI 折叠规则?**
spec 允许"订外卖"也是 task,理论上 UI 一天可能 30+ task。需要折叠默认值:
- 默认只展开 effective_state='active' 的 task
- effective_state='paused' 折叠成 "X paused"
- done/dropped 默认隐藏,日期 navigator 切到历史日才展开

但折叠粒度("近 24h paused" vs "近 7d paused")需要 UI dogfood 决定。**Step 4 上线后看用户反馈再调**。

### 11.3 隐性假设(写明白防漂)

下列假设 v1 接受,但要 **显式写下来** 防止后续不知不觉违反或扩展:

- **单用户**:整个 schema 没 `user_id` 概念,跨设备共享 / 多用户协作目前 **不在设计内**。v2/v3 若要做,得整体加 owner 维度
- **单时区**:`DbInstant` 全 UTC,但 logical day(05:00 切)、work_schedule 时段都按 **采集时本地时区** 隐式解读。用户跨时区(出差/搬家)后,旧 events 的"这是不是工作时段"判断会 **歪**。v1 接受,v2 议题
- **本地优先**:所有数据本地 SQLite。如果加云同步,about_user / tags 是要 sync 的"关于用户的知识",time_segments 是派生不需要 sync,events 是 SSOT 但量大要分批 — 现在都没考虑,加同步时是另一份 spec
- **macOS only**:capture / accessibility / 通知层都是 macOS-specific。Windows / Linux 移植 v3+ 才碰

---

## 12. 与现有架构的接合面

### 12.1 删除的部分(破坏性)

| 删除对象 | 替代物 |
|---|---|
| `projects` 表 | `tags` 表 namespace='project' |
| `project_identity_anchors` 表 | **拆成** `tag_anchors` + `task_anchors` 两张表(无 polymorphic FK) |
| `tasks.project_id` 字段 | `task_tags` 多对多 |
| `tasks.active_until` 字段(draft 一度有,本版直接不要) | `tasks_with_state` view 派生 effective_state |
| `tasks` 的 paused 存储态 | view 派生(`last_event_at` 30 min 阈值) |
| `services/event_attributor` | `services/task_attributor` |
| `prompts/event_task_attribute.md` | `prompts/task_attribute.md` |
| `prompts/project_rollup.md` | `prompts/tag_rollup.md` |

### 12.2 增量改的部分

| 文件 | 改动 |
|---|---|
| `db/schema.sql` | 大改:删两张,加五张(tags / task_tags / tag_anchors / task_anchors / time_segments / about_user / about_user_fts) + 1 view (tasks_with_state),改 tasks |
| `db/migrations.rs` | TARGET_SCHEMA_VERSION = 200,扩展 purge 列表 |
| `services/observation_session/prompts.rs` | 删 kind,保留 verbatim/anti-cross/regularity_signal |
| `services/retrospective_reviewer/mod.rs` | 加三个新 pass 模块 |
| `commands/workspace.rs` | 大改:tag CRUD + namespace_breakdown + active_tasks(返回 effective_state) + reopen_task |
| `prompts/event_boundary.md` | 简化(去 kind 段) |
| `prompts/retrospective_reassign.md` | 候选改成 effective_state ∈ {active, paused} 的 tasks |
| `src/pages/now/...` | 三栏改造 + view switch(Project / Timeline 双子视图,单页面 toggle) |
| `src/pages/settings/...` | 加 About-User / Work Schedule / Tags 三个标签页 |
| `src/lib/types.ts` | 加 Tag / TaskTag / TagAnchor / TaskAnchor / TaskWithState / AboutUserEntry / TimeSegment |

### 12.3 新增的部分

- `services/task_attributor/` 整个目录(替代 event_attributor)
- `services/tag_assignor/` 整个目录
- `services/about_user/` 整个目录
- `services/tag_narrator/` 整个目录
- `prompts/task_attribute.md`、`tag_assign.md`、`tag_review.md`、`temporal_segment.md`、`user_profile_distill.md`、`tag_rollup.md`

---

## 13. 决策日志

记录到 [design-decisions.md](design-decisions.md) 的关键决策：

- **D-T1**:task 升级为骨架,project 降级为可选标签,删除 projects 表与 P/T/E 三层结构
- **D-T2**:tags 表用 namespace 区分语义(project / domain / topic);同 namespace 内 unique(namespace, name)
- **D-T3**:**anchor 拆两张表** —— `tag_anchors`(长寿,跟随 tag) + `task_anchors`(短寿,跟随 task),不做 polymorphic FK。理由:owner 生命周期不同,polymorphic 牺牲外键完整性,而拆表只多一个 schema 项,换来真正的 ON DELETE CASCADE + 索引更紧
- **D-T4**:attribution 走 task-first:event 先决定并入哪个候选 task(候选 = `effective_state ∈ {active, paused}` 且 `last_event_at` 在 4h 内),task 形成后再由 tag_assignor 打 tag
- **D-T5**:多 active task 并行是常态,不是异常
- **D-T6**:events 不加 kind 字段。"工作 / 休闲" 由 task 是否挂 namespace='project' 的 tag 表达
- **D-T7**:events 是 SSOT,task 是物化语义投影,time_segments 是派生时间投影,about_user 是闭环反思层(核心架构判断)
- **D-T8**:合并 Memory + Knowledge 为 about_user 扁平表,kind 字段开放扩展
- **D-T9**:用户 override(move_event / change_tag)是高优先级写保护信号,触发 about_user distill
- **D-T10**:task 完成态 / standalone event 都是合法终态,不进 retrospective 反复重试
- **D-T11**:**`paused` 不入库,改派生**。`tasks.state` 收窄为 `{active, done, dropped}`,`effective_state` 由 `tasks_with_state` view 算(`last_event_at + 30min` 阈值)。理由:派生量入表会引入"翻状态 timer + task_resumption trigger"两层不变量,删了之后新 event 落入直接更新 `last_event_at`,view 自动算回 active,无副作用
- **D-T12**:**Open Questions 收窄为 4 条真问题**。原 11 条里 7 条挂"凭直觉,dogfood 验证"的悬空项(active 阈值、break 阈值、logical day 切点、task 自动 done、anchor 重算策略、time_segments 跨夜、schema 版本号)统一锁 default,只留 4 条需要真实数据回答的(namespace 多 tag 是否硬约束、tag 拆分时 anchor 怎么分、anchor 漂移检测指标、task 折叠粒度)。"留空等数据"是修补思维,Day One 思维下先决定有理据的 default,验证后再调
- **D-T13**:**§14 prompt 工程细则上提合入 §5**。原 spec 把 prompt 通则塞在末章,跟具体 prompt 章节互相引用 ── prompt 契约应当跟数据模型同章定义,不应散落两处
