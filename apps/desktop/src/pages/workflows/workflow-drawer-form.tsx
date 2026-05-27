import { useEffect, useMemo, useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import type {
  Trigger,
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
  /** Existing workflow being edited. Mutually exclusive with `presetId`. */
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
    intent: view.definition.system_prompt,
    reminderText: "",
  }
}

/**
 * Stage-2 form. Surface is uniform across built-in presets and custom
 * workflows: name + (intent / reminder if applicable) + when + notify
 * + enabled. Technical fields (slug, system_prompt, tool_whitelist,
 * max_turns) are pre-filled from the preset on create and preserved
 * from the existing definition on edit — they never appear in the UI.
 * Power users who need to tweak them edit the WORKFLOW.md on disk.
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
  // Auto-derive slug from name; slug is never exposed in the UI but
  // the backend needs a valid one.
  useEffect(() => {
    if (editing) return
    const derived = kebabFromName(form.name)
    setForm((prev) => ({ ...prev, slug: derived }))
  }, [form.name, editing])

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

      {/* Description — only in edit mode. Create mode either uses a
          preset's pre-filled description verbatim, or the user is
          writing an intent/reminder below which describes what the
          workflow does. */}
      {isEdit ? (
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
      ) : null}

      {/* Intent / reminder text depending on preset (create only). */}
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
