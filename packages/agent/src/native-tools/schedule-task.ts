// Native AgentTools: schedule_task / list_scheduled_tasks /
// cancel_scheduled_task / update_scheduled_task (v1431).
//
// Lets the agent author cron-style reminders mid-conversation.
// Storage is shared with the user-facing /workflows page — agent-
// created tasks land with source='agent' and the UI badges them so
// the user can audit and prune.
//
// Wire shape mirrors `WorkflowSaveSpec` in the Rust domain types;
// keep this file's parameter schemas in sync with
// `apps/desktop/src-tauri/src/domain/workflow.rs` and the handlers
// in `apps/desktop/src-tauri/src/services/exec_agent/schedule_task_handler.rs`.

import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";
import { isMockMode } from "../mock.js";

const Weekday = Type.Union(
  [
    Type.Literal("mon"),
    Type.Literal("tue"),
    Type.Literal("wed"),
    Type.Literal("thu"),
    Type.Literal("fri"),
    Type.Literal("sat"),
    Type.Literal("sun"),
  ],
  {
    description:
      "Weekday identifier; matches chrono::Weekday on the Rust side.",
  },
);

/**
 * Tagged-union trigger schema. The LLM picks one variant by setting
 * `kind`; the Rust validator (`Trigger::validate`) rejects bad shapes
 * before anything hits the DB.
 */
const Trigger = Type.Union(
  [
    Type.Object({
      kind: Type.Literal("interval"),
      minutes: Type.Number({
        minimum: 1,
        description:
          "Fire every N minutes after the previous run (or after creation on the very first run).",
      }),
    }),
    Type.Object({
      kind: Type.Literal("daily"),
      hour: Type.Number({ minimum: 0, maximum: 23 }),
      minute: Type.Number({ minimum: 0, maximum: 59 }),
      tz: Type.String({
        description:
          "IANA timezone, e.g. 'Asia/Shanghai'. If unsure, ask the user or default to their previously stated tz.",
      }),
    }),
    Type.Object({
      kind: Type.Literal("weekly"),
      weekdays: Type.Array(Weekday, { minItems: 1 }),
      hour: Type.Number({ minimum: 0, maximum: 23 }),
      minute: Type.Number({ minimum: 0, maximum: 59 }),
      tz: Type.String(),
    }),
    Type.Object({
      kind: Type.Literal("once"),
      at: Type.String({
        description:
          "ISO 8601 UTC instant, e.g. '2026-05-22T10:00:00Z'. Past instants are rejected.",
      }),
    }),
    Type.Object({
      kind: Type.Literal("cron"),
      expr: Type.String({
        description:
          "Standard 5-field cron expression: 'minute hour day-of-month month day-of-week'. Prefer one of the structured kinds when possible.",
      }),
      tz: Type.String(),
    }),
  ],
  {
    description:
      "When the task should fire. Pick the simplest kind that matches the user's intent.",
  },
);

// ---------------------------------------------------------------------------
// schedule_task
// ---------------------------------------------------------------------------

const ScheduleParameters = Type.Object({
  name: Type.String({
    description:
      "Short human-readable name shown in the /workflows UI. The slug is auto-derived; don't try to encode kebab-case here.",
  }),
  prompt: Type.String({
    description:
      "System prompt the scheduled run uses. Spell out what to do — assume future-you only has this and the tools you whitelist. Supports {{date}} and {{date_yesterday}} placeholders, substituted at run time.",
  }),
  trigger: Trigger,
  description: Type.Optional(
    Type.String({
      description:
        "Optional one-line description shown in the UI list and the run history.",
    }),
  ),
  tools: Type.Optional(
    Type.Array(Type.String(), {
      description:
        "Native tool names the scheduled run is allowed to call. Defaults to a broad read-oriented set: 'memory_search', 'save_note', 'recall_screen_history', 'thread_search', 'chat_thread_get', 'note_list'. Trim if you have a clear reason; add 'read'/'grep'/'find'/'ls' for filesystem-aware workflows. Avoid 'write'/'edit'/'bash' unless the user clearly asked the agent to act on its own.",
    }),
  ),
  max_turns: Type.Optional(
    Type.Number({
      minimum: 1,
      maximum: 50,
      description: "Cap on tool-use rounds per fire. Defaults to 12.",
    }),
  ),
  enabled: Type.Optional(
    Type.Boolean({
      description:
        "Whether the schedule fires automatically. Defaults to true — agent-created tasks should usually start enabled because the user just asked for them. Set false when the user says 'set this up but don't run it yet'.",
    }),
  ),
  notify: Type.Optional(
    Type.Union(
      [
        Type.Literal("always"),
        Type.Literal("on_change"),
        Type.Literal("silent"),
      ],
      {
        description:
          "How loudly to notify the user after each run. 'always' (default) fires a macOS banner + in-app toast every time. 'on_change' only fires when the output differs from the previous run (good for daily summaries that often have nothing new). 'silent' never pushes — the run still appears in the sidebar 'Corivo 提议' section. Pick 'always' for explicit reminders the user asked for, 'on_change' for ambient summaries, 'silent' for cleanup chores the user doesn't want to be interrupted by.",
      },
    ),
  ),
});

interface ScheduleResult {
  slug: string;
  enabled: boolean;
  next_run_at: string | null;
}

export const scheduleTaskTool: AgentTool<typeof ScheduleParameters> = {
  name: "schedule_task",
  label: "建定时任务",
  description:
    "Create a scheduled task that fires the agent later — daily review, weekly summary, one-shot reminder, etc. The scheduled run gets its own system prompt + tool whitelist and the user can see / edit / disable it from the /workflows page. Use when the user says 提醒 / 每天 / 每周 / 帮我定时 / set up / remind me. Don't use for one-shot in-chat tasks.",
  parameters: ScheduleParameters,
  execute: async (_toolCallId, params, signal) => {
    let result: ScheduleResult;
    if (isMockMode()) {
      result = {
        slug: "mock-task",
        enabled: params.enabled ?? true,
        next_run_at: null,
      };
    } else {
      result = (await rustRpc(
        "schedule_task",
        params,
        signal,
      )) as ScheduleResult;
    }
    const nextLine = result.next_run_at
      ? `下次将在 ${result.next_run_at}`
      : "未排出未来时间(检查触发器)";
    const summary = `已创建工作流 \`${result.slug}\` (${
      result.enabled ? "已启用" : "未启用"
    }); ${nextLine}`;
    return { content: [{ type: "text", text: summary }], details: result };
  },
};

// ---------------------------------------------------------------------------
// list_scheduled_tasks
// ---------------------------------------------------------------------------

const ListParameters = Type.Object({
  scope: Type.Optional(
    Type.Union(
      [Type.Literal("all"), Type.Literal("user"), Type.Literal("agent")],
      {
        description:
          "Restrict to user-created or agent-created tasks. Defaults to 'all'.",
      },
    ),
  ),
});

interface ListResultEntry {
  slug: string;
  name: string;
  description: string | null;
  trigger: unknown;
  enabled: boolean;
  source: "user" | "agent";
  last_run_at: string | null;
  next_run_at: string | null;
  last_status: string | null;
}
interface ListResult {
  tasks: ListResultEntry[];
}

export const listScheduledTasksTool: AgentTool<typeof ListParameters> = {
  name: "list_scheduled_tasks",
  label: "看定时任务",
  description:
    "List every scheduled task the user has — both user-authored ones (from the /workflows page) and ones you created earlier via schedule_task. Use when the user asks '我设了哪些提醒' / 'what reminders do I have'.",
  parameters: ListParameters,
  execute: async (_toolCallId, params, signal) => {
    let result: ListResult;
    if (isMockMode()) {
      result = { tasks: [] };
    } else {
      result = (await rustRpc(
        "list_scheduled_tasks",
        params,
        signal,
      )) as ListResult;
    }
    const count = result.tasks.length;
    return {
      content: [{ type: "text", text: `查到 ${count} 个定时任务` }],
      details: result,
    };
  },
};

// ---------------------------------------------------------------------------
// cancel_scheduled_task
// ---------------------------------------------------------------------------

const CancelParameters = Type.Object({
  slug: Type.String({
    description:
      "Slug of the scheduled task to cancel. Get it from `list_scheduled_tasks` if you don't have it.",
  }),
});

interface CancelResult {
  cancelled: boolean;
  slug?: string;
  reason?: string;
}

export const cancelScheduledTaskTool: AgentTool<typeof CancelParameters> = {
  name: "cancel_scheduled_task",
  label: "取消定时任务",
  description:
    "Delete a scheduled task by slug. Removes both the WORKFLOW.md and the schedule row — run history goes too. Confirm with the user before calling this if you're not sure they want it gone permanently.",
  parameters: CancelParameters,
  execute: async (_toolCallId, params, signal) => {
    let result: CancelResult;
    if (isMockMode()) {
      result = { cancelled: true, slug: params.slug };
    } else {
      result = (await rustRpc(
        "cancel_scheduled_task",
        params,
        signal,
      )) as CancelResult;
    }
    const text = result.cancelled
      ? `已取消 \`${params.slug}\``
      : `未找到 \`${params.slug}\`${result.reason ? ` (${result.reason})` : ""}`;
    return { content: [{ type: "text", text }], details: result };
  },
};

// ---------------------------------------------------------------------------
// update_scheduled_task
// ---------------------------------------------------------------------------

const UpdateParameters = Type.Object({
  slug: Type.String({
    description: "Slug of the task to update.",
  }),
  // Partial update — every other field is optional and falls back to
  // the row's current value if omitted.
  name: Type.Optional(Type.String()),
  description: Type.Optional(Type.String()),
  prompt: Type.Optional(Type.String()),
  trigger: Type.Optional(Trigger),
  tools: Type.Optional(Type.Array(Type.String())),
  max_turns: Type.Optional(Type.Number({ minimum: 1, maximum: 50 })),
  enabled: Type.Optional(Type.Boolean()),
  notify: Type.Optional(
    Type.Union(
      [
        Type.Literal("always"),
        Type.Literal("on_change"),
        Type.Literal("silent"),
      ],
      {
        description:
          "Patch the notification policy. Same semantics as `schedule_task`'s `notify`. Omit to leave unchanged.",
      },
    ),
  ),
});

interface UpdateResult {
  slug: string;
  enabled: boolean;
  next_run_at: string | null;
}

export const updateScheduledTaskTool: AgentTool<typeof UpdateParameters> = {
  name: "update_scheduled_task",
  label: "改定时任务",
  description:
    "Patch fields on an existing scheduled task. Send only the fields that change — the rest keep their current values. Use for 'change the daily review to 8am' / 'add screen-history search to the morning recap'.",
  parameters: UpdateParameters,
  execute: async (_toolCallId, params, signal) => {
    let result: UpdateResult;
    if (isMockMode()) {
      result = {
        slug: params.slug,
        enabled: params.enabled ?? true,
        next_run_at: null,
      };
    } else {
      result = (await rustRpc(
        "update_scheduled_task",
        params,
        signal,
      )) as UpdateResult;
    }
    const nextLine = result.next_run_at
      ? `; 下次 ${result.next_run_at}`
      : "";
    return {
      content: [
        {
          type: "text",
          text: `已更新 \`${result.slug}\` (${
            result.enabled ? "启用" : "禁用"
          })${nextLine}`,
        },
      ],
      details: result,
    };
  },
};
