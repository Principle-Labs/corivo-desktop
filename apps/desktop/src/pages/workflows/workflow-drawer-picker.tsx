import { useTranslation } from "@/i18n"
import { PRESETS, type PresetId } from "@/pages/workflows/presets"

interface Props {
  onPick: (id: PresetId) => void
}

/**
 * Picker stage of the create drawer — 4 cards that map to slim,
 * pre-filled forms. The "自己想一个" card opens a blank custom form;
 * the other three pre-fill name / prompt / tools / trigger and only
 * ask the user for time + notify policy.
 *
 * Cards are intentionally large (text + blurb) so the user reads each
 * before clicking. Dense lists trained users to scan-and-miss; cards
 * force a beat of consideration.
 */
export function WorkflowDrawerPicker({ onPick }: Props) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-4">
      <p className="text-[12.5px] text-muted-foreground">
        {t.workflows.picker.intro}
      </p>
      <div className="grid grid-cols-2 gap-2.5">
        {PRESETS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            onClick={() => onPick(preset.id)}
            className="flex flex-col items-start gap-1.5 rounded-lg border border-border/40 bg-card p-3.5 text-left transition-colors hover:border-foreground/30 hover:bg-muted/30"
          >
            <span className="text-[18px] leading-none">{preset.icon}</span>
            <span className="font-display text-[13.5px] font-medium tracking-[-0.005em] text-foreground">
              {preset.label}
            </span>
            <span className="line-clamp-2 text-[11.5px] leading-[1.5] text-muted-foreground">
              {preset.blurb}
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}
