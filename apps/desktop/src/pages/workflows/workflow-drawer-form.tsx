import { useEffect, useMemo, useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import type {
  Trigger,
  WorkflowNotifyPolicy,
  WorkflowSaveSpec,
  WorkflowView,
} from "@corivo/shared-types"
import { toast } from "sonner"

import { Button } from "@repo/ui/components/button"
import { Input } from "@repo/ui/components/input"
import { Label } from "@repo/ui/components/label"
import { Textarea } from "@repo/ui/components/textarea"

import { useTranslation } from "@/i18n"
import { fromInvokeError, workflowsSave } from "@/lib/tauri"
import {
  defaultsForKind,
  TriggerPicker,
} from "@/pages/workflows/trigger-picker"
import {
  findPreset,
  PRESETS,
  type PresetId,
  type WorkflowPreset,
} from "@/pages/workflows/presets"

interface Props {
  /** Preset the user picked, or `null` when editing an existing workflow. */
  presetId: PresetId | null
  /** Existing workflow being edited. Mutually exclusive with `presetId` —
   *  edit mode always starts in "advanced visible" since the slug + raw
   *  fields are already meaningful. */
  editing: WorkflowView | null
  /** Slug collision check for create mode. */
  existingSlugs: string[]
  onClose: () => void
  onBackToPicker: () => void
}

interface FormState {
  slug: string
  name: string
  description: string
  systemPrompt: string
  toolsText: string
  maxTurns: number
  trigger: Trigger
  enabled: boolean
  notifyPolicy: WorkflowNotifyPolicy
  /** Custom-preset-only: the user's free-text "做什么" lives here and
   *  becomes `systemPrompt` verbatim on save. */
  intent: string
  /** Once-preset-only: the user-typed reminder line, substituted into
   *  the systemPrompt template's `{{reminder_text}}` slot. */
  reminderText: string
}

function emptyFormForPreset(preset: WorkflowPreset): FormState {
  return {
    slug: "",
    name: preset.name,
    description: preset.description,
    systemPrompt: preset.systemPrompt,
    toolsText: preset.tools.join(", "),
    maxTurns: preset.maxTurns,
    trigger: preset.trigger,
    enabled: true,
    notifyPolicy: preset.notifyPolicy,
    intent: "",
    reminderText: "",
  }
}

function formFromView(view: WorkflowView): FormState {
  return {
    slug: view.definition.slug,
    name: view.definition.name,
    description: view.definition.description ?? "",
    systemPrompt: view.definition.system_prompt,
    toolsText: view.definition.tool_whitelist.join(", "),
    maxTurns: view.definition.max_turns,
    trigger: view.schedule?.trigger ?? defaultsForKind("daily"),
    enabled: view.schedule?.enabled ?? true,
    notifyPolicy: view.definition.notify_policy,
    intent: view.definition.system_prompt,
    reminderText: "",
  }
}

/**
 * Stage-2 form. Shape depends on which preset was picked:
 *
 *   - `daily-review` / `weekly-summary` — name preset + trigger
 *     picker (kind locked to "daily" / "weekly") + notify radio +
 *     立即启用. systemPrompt / tools / max_turns are pre-filled but
 *     only visible via the advanced toggle.
 *
 *   - `once` — reminder textarea + one-shot datetime picker. Same
 *     advanced toggle.
 *
 *   - `custom` — name + intent textarea (becomes systemPrompt) +
 *     trigger picker (all 4 structured kinds) + notify radio +
 *     立即启用. Slug auto-derived from name; advanced toggle exposes
 *     it.
 *
 *   - editing — advanced is always visible; the picker view never
 *     opens this branch.
 */
export function WorkflowDrawerForm({
  presetId,
  editing,
  existingSlugs,
  onClose,
  onBackToPicker,
}: Props) {
  const { t } = useTranslation()
  const qc = useQueryClient()

  const preset = useMemo<WorkflowPreset | null>(
    () => (presetId ? findPreset(presetId) : null),
    [presetId],
  )

  const [form, setForm] = useState<FormState>(() => {
    if (editing) return formFromView(editing)
    if (preset) return emptyFormForPreset(preset)
    return emptyFormForPreset(findPreset("custom"))
  })
  // Edit mode always shows advanced; create mode starts hidden.
  const [showAdvanced, setShowAdvanced] = useState(!!editing)

  // Auto-derive slug from name for create-mode users so they never see
  // the field unless they open advanced.
  useEffect(() => {
    if (editing) return
    if (showAdvanced) return // user is editing slug manually
    const derived = kebabFromName(form.name)
    setForm((prev) => ({ ...prev, slug: derived }))
  }, [form.name, editing, showAdvanced])

  const save = useMutation({
    mutationFn: workflowsSave,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["workflows-list"] })
      onClose()
    },
    onError: (error) => toast.error(fromInvokeError(error)),
  })

  const isEdit = !!editing
  const isCustom = presetId === "custom"
  const isReminder = presetId === "once"
  const slugCollision =
    !isEdit && form.slug && existingSlugs.includes(form.slug.trim())

  const onSave = () => {
    // Custom preset: substitute the user's free-text intent into the
    // wrapper template's `{{intent}}` slot. The wrapper hides all the
    // tool / agent-loop boilerplate the user shouldn't have to think
    // about. Once preset: same shape, different slot name.
    let systemPrompt = form.systemPrompt
    if (isCustom) {
      systemPrompt = form.systemPrompt.replace("{{intent}}", form.intent)
    } else if (isReminder) {
      systemPrompt = form.systemPrompt.replace(
        "{{reminder_text}}",
        form.reminderText,
      )
    }

    const spec: WorkflowSaveSpec = {
      slug: form.slug.trim(),
      name: form.name.trim(),
      description: form.description.trim() ? form.description.trim() : null,
      tool_whitelist: parseToolList(form.toolsText),
      max_turns: form.maxTurns,
      system_prompt: systemPrompt,
      trigger: form.trigger,
      enabled: form.enabled,
      notify_policy: form.notifyPolicy,
    }
    save.mutate(spec)
  }

  const canSave =
    !save.isPending &&
    !!form.name.trim() &&
    !!form.slug.trim() &&
    !slugCollision &&
    (isCustom
      ? !!form.intent.trim()
      : isReminder
        ? !!form.reminderText.trim()
        : !!form.systemPrompt.trim())

  return (
    <div className="flex flex-1 flex-col gap-4">
      {!isEdit && preset ? (
        <button
          type="button"
          onClick={onBackToPicker}
          className="self-start text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          {t.workflows.picker.backToPicker}
        </button>
      ) : null}

      {/* Name — visible in all modes. For preset paths it starts
          pre-filled (e.g. "每日回顾") and most users won't edit it. */}
      <FieldRow>
        <Label>{t.workflows.drawer.name}</Label>
        <Input
          value={form.name}
          onChange={(e) =>
            setForm((prev) => ({ ...prev, name: e.target.value }))
          }
        />
      </FieldRow>

      {/* Intent / reminder text / system prompt depending on preset. */}
      {isCustom ? (
        <FieldRow>
          <Label>{t.workflows.simple.intentLabel}</Label>
          <Textarea
            value={form.intent}
            rows={5}
            placeholder="每天早上拉昨天的 frames，写一份纪要存到 notes。"
            onChange={(e) =>
              setForm((prev) => ({ ...prev, intent: e.target.value }))
            }
          />
          <p className="text-[10.5px] text-muted-foreground/70">
            {t.workflows.simple.intentHint}
          </p>
        </FieldRow>
      ) : isReminder ? (
        <FieldRow>
          <Label>{t.workflows.simple.reminderTextLabel}</Label>
          <Textarea
            value={form.reminderText}
            rows={2}
            placeholder="该回 Linear 上那条评论了"
            onChange={(e) =>
              setForm((prev) => ({ ...prev, reminderText: e.target.value }))
            }
          />
          <p className="text-[10.5px] text-muted-foreground/70">
            {t.workflows.simple.reminderTextHint}
          </p>
        </FieldRow>
      ) : null}

      <FieldRow>
        <Label>{t.workflows.simple.whenLabel}</Label>
        <TriggerPicker
          trigger={form.trigger}
          onChange={(trigger) =>
            setForm((prev) => ({ ...prev, trigger }))
          }
        />
      </FieldRow>

      <FieldRow>
        <Label>{t.workflows.drawer_notify.label}</Label>
        <NotifyPolicyPicker
          value={form.notifyPolicy}
          onChange={(notifyPolicy) =>
            setForm((prev) => ({ ...prev, notifyPolicy }))
          }
        />
      </FieldRow>

      <label className="flex items-center gap-2 text-[12.5px] text-foreground">
        <input
          type="checkbox"
          checked={form.enabled}
          onChange={(e) =>
            setForm((prev) => ({ ...prev, enabled: e.target.checked }))
          }
        />
        {t.workflows.simple.enabledLabel}
      </label>

      {/*
        Advanced toggle is intentionally hidden in CREATE mode. The
        whole point of the simple form is that the user expresses
        intent and the system handles tools / prompts / max_turns —
        even displaying a "显示完整字段" button plants the idea that
        there's something they should be thinking about. Power users
        who want to tweak an existing workflow can do so from EDIT
        mode (which is what `isEdit` gates below).
      */}
      {isEdit ? (
        <button
          type="button"
          onClick={() => setShowAdvanced((v) => !v)}
          className="self-start text-[11px] text-muted-foreground hover:text-foreground"
        >
          {showAdvanced
            ? t.workflows.simple.advancedHide
            : t.workflows.simple.advancedToggle}
        </button>
      ) : null}

      {isEdit && showAdvanced ? (
        <div className="flex flex-col gap-3 rounded-md border border-dashed border-border/40 p-3">
          <FieldRow>
            <Label>{t.workflows.drawer.slug}</Label>
            <Input
              value={form.slug}
              onChange={(e) =>
                setForm((prev) => ({ ...prev, slug: e.target.value }))
              }
              disabled={isEdit}
              className="font-mono"
            />
            {slugCollision ? (
              <p className="text-[10.5px] text-destructive">slug 已存在</p>
            ) : null}
          </FieldRow>
          <FieldRow>
            <Label>{t.workflows.drawer.description}</Label>
            <Input
              value={form.description}
              onChange={(e) =>
                setForm((prev) => ({
                  ...prev,
                  description: e.target.value,
                }))
              }
            />
          </FieldRow>
          {!isCustom && !isReminder ? (
            <FieldRow>
              <Label>{t.workflows.drawer.systemPrompt}</Label>
              <Textarea
                value={form.systemPrompt}
                rows={6}
                onChange={(e) =>
                  setForm((prev) => ({
                    ...prev,
                    systemPrompt: e.target.value,
                  }))
                }
                className="font-mono text-[11.5px]"
              />
            </FieldRow>
          ) : null}
          <FieldRow>
            <Label>{t.workflows.drawer.tools}</Label>
            <Textarea
              value={form.toolsText}
              rows={2}
              onChange={(e) =>
                setForm((prev) => ({ ...prev, toolsText: e.target.value }))
              }
              className="font-mono text-[11.5px]"
            />
          </FieldRow>
          <FieldRow>
            <Label>{t.workflows.drawer.maxTurns}</Label>
            <Input
              type="number"
              min={1}
              max={50}
              value={form.maxTurns}
              onChange={(e) =>
                setForm((prev) => ({
                  ...prev,
                  maxTurns: Math.max(
                    1,
                    parseInt(e.target.value || "1", 10) || 1,
                  ),
                }))
              }
              className="w-24"
            />
          </FieldRow>
        </div>
      ) : null}

      <div className="mt-auto flex justify-end gap-2 pt-2">
        <Button type="button" variant="ghost" onClick={onClose}>
          {t.workflows.drawer.cancelAction}
        </Button>
        <Button type="button" onClick={onSave} disabled={!canSave}>
          {t.workflows.drawer.saveAction}
        </Button>
      </div>
    </div>
  )
}

function FieldRow({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-col gap-1.5">{children}</div>
}

const NOTIFY_OPTIONS: ReadonlyArray<{
  value: WorkflowNotifyPolicy
  key: "always" | "onChange" | "silent"
}> = [
  { value: "always", key: "always" },
  { value: "on_change", key: "onChange" },
  { value: "silent", key: "silent" },
]

function NotifyPolicyPicker({
  value,
  onChange,
}: {
  value: WorkflowNotifyPolicy
  onChange: (next: WorkflowNotifyPolicy) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-1">
      {NOTIFY_OPTIONS.map((opt) => (
        <label
          key={opt.value}
          className={`flex cursor-pointer items-center gap-2 rounded-md border px-2 py-1.5 text-[12px] transition-colors ${
            value === opt.value
              ? "border-foreground/40 bg-foreground/5 text-foreground"
              : "border-border/40 text-muted-foreground hover:border-border/70"
          }`}
        >
          <input
            type="radio"
            name="notify-policy"
            value={opt.value}
            checked={value === opt.value}
            onChange={() => onChange(opt.value)}
            className="accent-foreground"
          />
          <span>{t.workflows.drawer_notify[opt.key]}</span>
        </label>
      ))}
    </div>
  )
}

function parseToolList(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

/**
 * Lower-case ASCII kebab from a human name. Strips CJK / punctuation
 * and falls back to "task" if the result is empty.
 *
 * Lives here (not in `format.ts`) because it's a form-specific
 * convenience for create-mode auto-slug; advanced editors should
 * type a real slug.
 */
function kebabFromName(name: string): string {
  const stripped = name
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "")
  if (stripped.length === 0) return "task"
  if (stripped.length > 48) return stripped.slice(0, 48).replace(/-+$/, "")
  // PRESETS list above implicitly hard-codes some slugs (e.g. for
  // "每日回顾" → "" → "task"). Suffix the timestamp tail so the user
  // doesn't immediately hit a collision with another preset card.
  if (PRESETS.some((p) => p.id === stripped)) {
    return `${stripped}-${Date.now().toString(36).slice(-3)}`
  }
  return stripped
}
