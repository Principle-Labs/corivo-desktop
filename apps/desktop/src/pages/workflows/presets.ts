import type { Trigger } from "@corivo/shared-types"

import { guessLocalTimezone } from "@/pages/workflows/format"

/**
 * Pre-filled workflow templates surfaced as cards in the create
 * drawer. Each preset hides the prompt-engineering layer: the user
 * only adjusts time + notification policy + (optionally) the visible
 * `intent` line, and the slug / tools / max_turns / system_prompt all
 * fall out of the template.
 *
 * The 4-preset list (per UX decision) — keep this short on purpose;
 * a long grid forces the user to read each card. If we add a 5th,
 * delete the weakest one rather than expanding.
 */

export type PresetId = "daily-review" | "weekly-summary" | "once" | "custom"

export interface WorkflowPreset {
  id: PresetId
  /** Card label. Short imperative phrase. */
  label: string
  /** Card body — one sentence describing what this does. */
  blurb: string
  /** Emoji-as-icon. Avoids pulling in 4 distinct lucide imports for
   *  the cards. */
  icon: string
  /** Default name (user can override in the form). */
  name: string
  /** Default description (user can override). */
  description: string
  /** System prompt template — fully baked. User doesn't see this in
   *  simple mode; advanced fold exposes it. */
  systemPrompt: string
  /** Native tools the runs may call. */
  tools: string[]
  /** Cap on tool-use rounds per fire. */
  maxTurns: number
  /** Suggested trigger. The form lets the user adjust HH:MM / day /
   *  interval but keeps the kind locked to what the preset implies. */
  trigger: Trigger
}

const TZ = guessLocalTimezone()

/**
 * System-prompt wrapper used when the user picks "自己想一个". The
 * `{{intent}}` slot is replaced at save time with whatever the user
 * typed in the "做什么" textarea — they never see this wrapper.
 *
 * Why a wrapper exists at all: an agent given only "请总结今天我做了
 * 什么" with no tool guidance will either hallucinate, infinite-think,
 * or give up. The wrapper tells it which tools are on the menu and
 * what shape the final response should take, so the user doesn't have
 * to be a prompt engineer.
 *
 * Keep the available-tools list in sync with the `tools:` array on
 * the `custom` preset (the runner intersects with the registry, so
 * extras here are silently dropped but visually misleading).
 */
export const CUSTOM_INTENT_TEMPLATE = `你是 Corivo 的自动化任务助手。

可用的工具:
- \`memory_search(query)\` — 查 Corivo 已经记录的屏幕历史、笔记、过往对话。需要"昨天我看了什么 / 上周聊了哪些项目"这类信息时优先用它。
- \`chat_thread_get(thread_id)\` — 读取某次具体对话的完整内容。
- \`note_list(scope, status)\` — 列出已经存在的笔记/偏好。
- \`thread_search(query)\` — 按关键词搜索过去的对话。
- \`save_note(content, scope, source)\` — 把结果存进长期记忆;以后 memory_search 能召回。

用户希望你做的事:

{{intent}}

执行规则:
1. 必要时主动调用上面的工具拿数据,不要凭空编。
2. 工具调用次数控制在 5 次以内,够用就停。
3. 把要告诉用户的最终结果作为最后一段输出。简明扼要,300 字以内,中文。
4. 如果用户的目标包含"记下 / 存下 / 整理到笔记"这类意图,务必调 \`save_note\` 真正写进去。
5. 如果没有需要执行的内容(比如时间窗口内没有任何活动),返回一句简短说明,不要硬凑。
`;

export const PRESETS: WorkflowPreset[] = [
  {
    id: "daily-review",
    label: "每日回顾",
    blurb: "每天早上自动拉昨天看了什么、做了什么，写一份纪要。",
    icon: "📋",
    name: "每日回顾",
    description: "拉取昨日的 frames 与对话，生成约 200 字的 Markdown 工作纪要并存入笔记。",
    systemPrompt: `你是 Corivo 的每日回顾助手。每次运行时，请按下面的步骤工作：

1. 用 \`memory_search\` 查询昨天（{{date_yesterday}}）的活动。
2. 阅读召回结果，挑出最值得记下的 3–5 件事（真正动手做了的事 / 重复出现的话题）。
3. 写一份 Markdown 纪要，结构：## 完成 / ## 进行中 / ## 想到的下一步，总长度约 200 字。
4. 用 \`save_note(content=<markdown>, scope="global", source="agent_inferred")\` 保存。
5. 把同样的内容作为最终回复返回。

没有可写内容时返回"昨日无显著活动"并跳过 save_note。`,
    tools: ["memory_search", "save_note"],
    maxTurns: 12,
    trigger: { kind: "daily", hour: 9, minute: 0, tz: TZ },
  },
  {
    id: "weekly-summary",
    label: "每周总结",
    blurb: "每周一早上汇总过去一周的活动，看清楚自己干了啥。",
    icon: "📅",
    name: "每周总结",
    description: "拉取过去 7 天的活动，按主题分类，输出周报式 Markdown 总结。",
    systemPrompt: `你是 Corivo 的每周总结助手。每次运行时：

1. 用 \`memory_search\` 多次查询过去 7 天的活动（不同关键词分别召回）。
2. 按主题聚类（项目 / 学习 / 对话 / 工具 / 其他）。
3. 写一份周报，结构：## 本周主线 / ## 看似零散但有联系的 / ## 下周想推进的，约 300–400 字。
4. 用 \`save_note\` 保存（scope="global", source="agent_inferred"）。
5. 把同样的内容作为最终回复返回。`,
    tools: ["memory_search", "save_note"],
    maxTurns: 15,
    trigger: { kind: "weekly", weekdays: ["mon"], hour: 10, minute: 0, tz: TZ },
  },
  {
    id: "once",
    label: "一次性提醒",
    blurb: "在指定时间叮你一下。最简单的『待会儿提醒我』。",
    icon: "⏰",
    name: "提醒",
    description: "在指定时间发出一条 macOS 通知。",
    systemPrompt: `你是 Corivo 的提醒助手。运行时，直接把下面这段提醒文本作为最终回复返回，不调用任何工具：

{{reminder_text}}

简短即可，正文不要超过 80 字。`,
    tools: [],
    maxTurns: 1,
    // Placeholder — the form will replace `at` with the user-picked
    // datetime before save. A new Date 1h ahead so the picker has a
    // sensible default if the user opens then immediately saves.
    trigger: {
      kind: "once",
      at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
    },
  },
  {
    id: "custom",
    label: "自己想一个",
    blurb: "用一句话告诉 Corivo 你要它做什么，时间也你定。",
    icon: "✨",
    name: "",
    description: "",
    // {{intent}} gets substituted at save time with whatever the
    // user typed in the "做什么" textarea. The wrapper hides all the
    // tool / agent-loop boilerplate the user shouldn't have to
    // think about — they just describe the outcome they want.
    systemPrompt: CUSTOM_INTENT_TEMPLATE,
    // Broad safe-read toolset + save_note for persistence. Covers
    // "总结过去 X / 回顾今天 / 找之前聊过 Y / 记一下 Z" — i.e. 90%
    // of agent-as-cron use cases — without exposing write/edit/bash
    // (filesystem mutation is a power-user surface and not what a
    // scheduled task should be doing by default).
    tools: [
      "memory_search",
      "chat_thread_get",
      "note_list",
      "thread_search",
      "save_note",
    ],
    maxTurns: 12,
    trigger: { kind: "daily", hour: 9, minute: 0, tz: TZ },
  },
]

export function findPreset(id: PresetId): WorkflowPreset {
  const found = PRESETS.find((p) => p.id === id)
  if (!found) {
    throw new Error(`unknown preset id: ${id}`)
  }
  return found
}
