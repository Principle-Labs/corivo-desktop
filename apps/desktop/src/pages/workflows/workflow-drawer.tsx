import { useEffect, useState } from "react"
import type { WorkflowView } from "@corivo/shared-types"

import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@repo/ui/components/sheet"

import { useTranslation } from "@/i18n"
import { WorkflowDrawerForm } from "@/pages/workflows/workflow-drawer-form"
import { WorkflowDrawerPicker } from "@/pages/workflows/workflow-drawer-picker"
import type { PresetId } from "@/pages/workflows/presets"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: WorkflowView | null
  existingSlugs: string[]
}

type Stage = { kind: "picker" } | { kind: "form"; presetId: PresetId }

/**
 * Create / edit drawer wrapper. Drives the two-stage flow:
 *
 *   1. **Picker** (create mode only) — 4 preset cards. The user picks
 *      one and we transition to the form stage with that preset
 *      pre-filled.
 *
 *   2. **Form** — slim form for the picked preset. Defaults hide the
 *      slug / system_prompt / tools / max_turns; an "显示完整字段"
 *      disclosure exposes them for power users.
 *
 * Edit mode skips the picker entirely — we open straight on the form
 * with `presetId = null` (the form falls back to whatever the existing
 * workflow has) and advanced is visible from the start.
 */
export function WorkflowDrawer({
  open,
  onOpenChange,
  editing,
  existingSlugs,
}: Props) {
  const { t } = useTranslation()
  const [stage, setStage] = useState<Stage>({ kind: "picker" })

  // Reset stage every time the drawer opens. Edit mode jumps past the
  // picker; create mode goes back to the picker so the user re-picks
  // if they reopen after a previous save.
  useEffect(() => {
    if (!open) return
    setStage(editing ? { kind: "form", presetId: "custom" } : { kind: "picker" })
  }, [open, editing])

  const isEdit = !!editing

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex flex-col gap-5 sm:max-w-lg">
        <SheetHeader>
          <SheetTitle>
            {isEdit
              ? t.workflows.drawer.titleEdit
              : t.workflows.drawer.titleCreate}
          </SheetTitle>
        </SheetHeader>

        {stage.kind === "picker" ? (
          <WorkflowDrawerPicker
            onPick={(presetId) => setStage({ kind: "form", presetId })}
          />
        ) : (
          <WorkflowDrawerForm
            presetId={isEdit ? null : stage.presetId}
            editing={editing}
            existingSlugs={existingSlugs}
            onClose={() => onOpenChange(false)}
            onBackToPicker={() => setStage({ kind: "picker" })}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}
