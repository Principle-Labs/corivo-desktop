---
name: corivo-create-workflow
description: 当用户表达想要创建定时/周期性/未来一次性任务（"每天 X"、"每周 X"、"下周 X"、"提醒我 X"、"帮我每天 Y"、"set up X"、"schedule X"、"remind me to X" 等）时使用。负责把用户的自然语言意图转换成完整的 schedule_task 调用，包含触发器、system prompt、工具白名单、通知策略。
---

# 创建定时工作流

当用户在对话里要求 Corivo 在未来某个时间或按某个频率执行任务时使用。最终调用的工具是 `schedule_task`。

## 触发场景

- **周期性**："每天 21 点提醒我整理笔记"、"每周一早上做周报"、"每隔半小时提醒喝水"
- **一次性**："下周二下午 3 点叫我开会"、"明天 9 点提醒我"
- **英文同义**：`remind me to X every day`、`schedule a daily/weekly Y`、`set up Z to fire at...`

## 不要使用本 skill 的情形

- 用户只是要现在做某事（"现在帮我搜一下…"、"总结今天对话"）→ 直接做
- 用户在询问已有任务（"我设了哪些提醒？"）→ 用 `list_scheduled_tasks`
- 用户在修改/删除现有任务 → 用 `update_scheduled_task` / `cancel_scheduled_task`

## 核心原则

**意图清晰就直接调用 `schedule_task`，不要为了"礼貌"反复确认。** 用户既然说了"帮我每天 9 点 X"，他要的就是它被设好，不是再问他一遍"确认设置吗"。只在信息**真的**缺失到无法构造合法参数时，才追问一句。

## 工作流程

### 1. 信息完整性检查

调用工具前，确认这两项明确：

- **要做什么** → 决定 `prompt` 和 `tools`
- **什么时候做** → 决定 `trigger`

任一项不明确，**只在必要时**问一句澄清。其余字段（描述、最大轮次、通知策略）都有合理默认，自己定就好。

### 2. 选 `trigger.kind`

| 用户语义 | `trigger.kind` | 必填字段 |
|---------|---------------|---------|
| "每天 X 点 / X:Y" | `daily` | `hour`, `minute`, `tz` |
| "每周 X / 工作日 X 点" | `weekly` | `weekdays`, `hour`, `minute`, `tz` |
| "明天 / 下周 / 具体日期 X 点" | `once` | `at`（UTC ISO 8601） |
| "每隔 N 分钟 / N 小时" | `interval` | `minutes` |
| 结构化 kind 表达不了的复杂规则 | `cron` | `expr`, `tz` |

**优先选结构化 kind**——除非用户明确要表达只有 cron 能描述的规则（如"每月第一个周一"），否则不用 cron。

**时区**：`daily` / `weekly` / `cron` 需要 `tz`。如果对话上下文没有时区线索，默认 `Asia/Shanghai`；不要为了问时区单独发一句话打断对话。

**`once.at`**：用户说本地时间（如"下周一下午 3 点"），换算成 UTC ISO 8601 instant 再填。绝不把本地时间字符串直接当 `at` 用。

### 3. 写 `prompt`

scheduled run 在未来运行时**只能看到** `prompt` 字段和你给的 `tools`——没有当前对话上下文。所以 prompt 必须自包含。

好的 prompt：

- 动词开头，明确要做什么
- 用 `{{date}}` / `{{date_yesterday}}` 占位符指代日期（运行时被替换为 "2026-05-25" 这种格式）
- 指明结果以什么形式呈现：保存到 note？发通知？返回总结？
- 提醒类的 prompt 直接就是要给用户看的提醒内容

### 4. 选 `tools`

默认 `["memory_search", "save_note", "recall_screen_history", "thread_search", "chat_thread_get", "note_list"]` 已覆盖绝大多数信息整理类工作流。**只在用户明确需要这个集合外的工具时**才在 `tools` 字段里显式列出。

**典型扩展**：

- 工作流要读本地文件 / 目录 → 加 `read`、`grep`、`find`、`ls`
- 用户明确要让 agent 自己改文件 / 跑命令 → 才加 `write`、`edit`、`bash`

倾向**宽**而不是窄——给未来的 scheduled run 足够能力，让它能完成用户描述的事。

### 5. 选 `notify`

- **`always`**（默认）：用户主动要的提醒、deadline、约会 → 每次响
- **`on_change`**：周期性总结、监控类 → 只在内容变了才通知（"昨天没什么新事"就别打扰）
- **`silent`**：自动归档 / 整理类，用户不想被打断但运行结果可以在侧栏的"Corivo 提议"里看到

### 6. 创建后

`schedule_task` 返回 `{ slug, enabled, next_run_at }`。**一句话**告诉用户：

- "好，每天 21:30 整理工作纪要，下次 2026-05-25 21:30 跑。"
- 不要复述 prompt 内容
- 不要追问"还要调吗"——用户想调自己会说

## 范例

### 范例 1：每日总结

**用户**："帮我每天晚上 9 点 30 整理一下今天做了什么"

**调用**：
```json
{
  "name": "每日回顾",
  "prompt": "用 memory_search 和 recall_screen_history 查 {{date}} 当天的活动，找出主要做了什么事，把约 200 字的 Markdown 工作纪要用 save_note 保存下来。",
  "trigger": { "kind": "daily", "hour": 21, "minute": 30, "tz": "Asia/Shanghai" },
  "description": "整理当天工作纪要",
  "tools": ["memory_search", "recall_screen_history", "save_note", "thread_search"],
  "notify": "on_change"
}
```

**回复**："好，每天 21:30 整理工作纪要，下次 2026-05-25 21:30。"

### 范例 2：一次性提醒

**用户**："下周二下午 3 点提醒我开周会"

**调用**：
```json
{
  "name": "周二周会提醒",
  "prompt": "提醒用户：下午 3 点的周会该开始了。",
  "trigger": { "kind": "once", "at": "2026-06-02T07:00:00Z" },
  "description": "周二周会提醒",
  "notify": "always"
}
```

**回复**："好，下周二（6/2）下午 3 点提醒你。"

### 范例 3：每周总结

**用户**："每周五下午 5 点帮我做个本周总结"

**调用**：
```json
{
  "name": "周总结",
  "prompt": "用 memory_search 和 thread_search 查本周（往前 7 天到 {{date}}）的活动和对话，写一份 300-500 字的 Markdown 周报用 save_note 保存。重点突出本周的主要事项、决策、待办。",
  "trigger": { "kind": "weekly", "weekdays": ["fri"], "hour": 17, "minute": 0, "tz": "Asia/Shanghai" },
  "description": "每周五下午做本周总结",
  "tools": ["memory_search", "thread_search", "save_note", "note_list"],
  "notify": "on_change"
}
```

**回复**："好，每周五 17:00 做本周总结。"

### 范例 4：间隔检查

**用户**："每隔一小时帮我看一下有没有新的会议邀请"

**调用**：
```json
{
  "name": "每小时检查会议邀请",
  "prompt": "用 thread_search 和 note_list 检查最近一小时内是否有新的会议相关消息或笔记。若有新内容，简短列出；若没有，silent。",
  "trigger": { "kind": "interval", "minutes": 60 },
  "description": "每小时扫一次会议邀请",
  "tools": ["thread_search", "note_list", "chat_thread_get"],
  "notify": "on_change"
}
```

**回复**："好，每小时检查一次会议邀请，第一次现在到点就跑。"

## 错误处理

- `Trigger::validate` 在 Rust 侧会拒绝错误形状（如 `hour > 23`、过去的 instant）。如果工具返回错误，**读错误信息、修正参数、重试**，不要直接放弃。
- 如果用户描述的时间过于模糊（"以后某时"、"看心情"、"想起来再说"），明确告诉用户："这个时间需要更具体一点，比如几点、几号"。
- 如果用户描述的事情过于模糊（"整理一下东西"、"搞点事"），追问一句"具体整理什么 / 做什么"。

## 与相邻 skill / 工具的边界

- 创建任务 → 本 skill + `schedule_task`
- 列出已有任务 → 直接用 `list_scheduled_tasks`，不需要 skill
- 修改任务 → 直接用 `update_scheduled_task`，不需要 skill
- 删除任务 → 用 `cancel_scheduled_task`，删除前简短确认（删比改更不可逆）
