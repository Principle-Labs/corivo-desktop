import { useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@repo/ui/components/dialog"
import { cn } from "@repo/ui/lib/utils"
import { useCapabilities } from "@/hooks/use-capabilities"
import { useConfig } from "@/hooks/use-config"
import { useTranslation } from "@/i18n"
import { AboutSection } from "@/pages/settings/sections/about-section"
import { CapturePrivacySection } from "@/pages/settings/sections/capture-privacy-section"
import { DeveloperSection } from "@/pages/settings/sections/developer-section"
import { ExecAgentSection } from "@/pages/settings/sections/exec-agent-section"
import { GeneralSection } from "@/pages/settings/sections/general-section"
import { IntegrationsSection } from "@/pages/settings/sections/integrations-section"
import { MemorySection } from "@/pages/settings/sections/memory-section"
import { QuickAskSection } from "@/pages/settings/sections/quick-ask-section"
import { ShortcutsSection } from "@/pages/settings/sections/shortcuts-section"
import { SkillsSection } from "@/pages/settings/sections/skills-section"
import {
  clearSettingsRequest,
  closeSettingsDialog,
  setSettingsDialogOpen,
  useSettingsDialogOpen,
  useSettingsRequestedSection,
} from "@/stores/settings-dialog-store"

type SectionId =
  | "general"
  | "capture"
  | "quick-ask"
  | "exec-agent"
  | "skills"
  | "memory"
  | "integrations"
  | "shortcuts"
  | "about"
  | "developer"

// Triple-tap unlock: three clicks within this window flip
// `Config.app.developer_mode` to true. 600 ms is forgiving enough for a
// human tapping deliberately but short enough that an accidental
// double-click followed by a real click a second later won't trigger.
const UNLOCK_TAP_WINDOW_MS = 600
const UNLOCK_REQUIRED_TAPS = 3

const KNOWN_SECTION_IDS: SectionId[] = [
  "general",
  "capture",
  "quick-ask",
  "exec-agent",
  "skills",
  "memory",
  "integrations",
  "shortcuts",
  "about",
  "developer",
]

function coerceSectionId(value: string | undefined): SectionId | null {
  if (!value) return null
  return KNOWN_SECTION_IDS.includes(value as SectionId)
    ? (value as SectionId)
    : null
}

export function SettingsDialog() {
  const { t } = useTranslation()
  const open = useSettingsDialogOpen()
  const requestedSection = useSettingsRequestedSection()
  const [active, setActive] = useState<SectionId>("general")
  const { config, update } = useConfig()

  // When a caller (the privacy popover's "Manage" button, today) opens
  // settings with a section target, jump to it once on transition into
  // open and clear the request. The clear is what makes a subsequent
  // manual open return to the user's last-active tab.
  useEffect(() => {
    if (!open) return
    const target = coerceSectionId(requestedSection)
    if (target) {
      setActive(target)
      clearSettingsRequest("section")
    }
  }, [open, requestedSection])
  // Tap counter is intentionally ephemeral: it lives in a ref so we don't
  // trigger re-renders on every click, and gets reset when the dialog
  // closes (via the open=false branch below). That makes the unlock
  // ritual one-shot per dialog session — closing and reopening starts
  // fresh.
  const unlockTapsRef = useRef(0)
  const lastTapAtRef = useRef(0)

  const developerMode = config?.app.developer_mode ?? false
  const { data: capabilities } = useCapabilities()
  const connectorsAvailable = capabilities?.connectors ?? false

  // Capture & Privacy was elevated to second-position in the redesign
  // (see docs/design/ia-v0.html §05); the old standalone Permissions
  // tab is now folded into it as a sub-card.
  //
  // Integrations 标签只在构建带 connectors capability 时存在 ——
  // 开源构建里 connector_registry 是 Noop，list() 永远返回空，所以
  // 标签整体藏掉而不是显示一个"什么都没有"的空 tab。
  const sections = useMemo<{ id: SectionId; label: string }[]>(() => {
    const base: { id: SectionId; label: string }[] = [
      { id: "general", label: t.settings.sections.general },
      { id: "capture", label: t.settings.sections.capture },
      { id: "quick-ask", label: t.settings.sections.quickAsk },
      { id: "exec-agent", label: t.settings.sections.execAgent },
      { id: "skills", label: t.settings.sections.skills },
      { id: "memory", label: t.settings.sections.memory },
    ]
    if (connectorsAvailable) {
      base.push({ id: "integrations", label: t.settings.sections.integrations })
    }
    base.push(
      { id: "shortcuts", label: t.settings.sections.shortcuts },
      { id: "about", label: t.settings.sections.about },
    )
    if (developerMode) {
      base.push({ id: "developer", label: t.settings.sections.developer })
    }
    return base
  }, [t, developerMode, connectorsAvailable])

  // If the user flips developer_mode off while inside the Developer tab,
  // bounce them back to General so the sidebar selection stays in sync
  // with what's rendered to the right.
  useEffect(() => {
    if (!developerMode && active === "developer") {
      setActive("general")
    }
  }, [developerMode, active])

  function handleTitleTap() {
    if (!config) return
    if (developerMode) {
      toast(t.settings.developerUnlock.alreadyOn)
      return
    }
    const now = performance.now()
    const within = now - lastTapAtRef.current <= UNLOCK_TAP_WINDOW_MS
    unlockTapsRef.current = within ? unlockTapsRef.current + 1 : 1
    lastTapAtRef.current = now

    if (unlockTapsRef.current >= UNLOCK_REQUIRED_TAPS) {
      unlockTapsRef.current = 0
      update((prev) => ({
        ...prev,
        app: { ...prev.app, developer_mode: true },
      }))
      toast.success(t.settings.developerUnlock.unlocked, {
        description: t.settings.developerUnlock.unlockedHint,
      })
      setActive("developer")
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setSettingsDialogOpen(true)
        } else {
          // Reset both the tap counter and the active tab so reopening
          // gives a clean state — the dialog is the only "session" the
          // unlock ritual cares about.
          unlockTapsRef.current = 0
          lastTapAtRef.current = 0
          closeSettingsDialog()
        }
      }}
    >
      <DialogContent
        className="flex h-[78vh] max-h-[680px] w-[92vw] max-w-[880px] flex-col gap-0 overflow-hidden p-0 sm:rounded-[14px]"
      >
        <div className="flex h-10 shrink-0 items-center justify-center border-b border-border bg-[color-mix(in_oklab,var(--foreground)_3%,var(--card))]">
          <button
            type="button"
            onClick={handleTitleTap}
            aria-label={t.settings.dialog.title}
            className="cursor-default select-none text-[12px] font-medium tracking-[-0.005em] text-muted-foreground outline-none"
          >
            {t.settings.dialog.title}
          </button>
        </div>
        <div className="grid min-h-0 flex-1 grid-cols-[196px_1fr] overflow-hidden">
          <aside className="flex h-full flex-col gap-[1px] overflow-y-auto border-r border-border bg-muted/40 p-2">
            <DialogTitle className="sr-only">
              {t.settings.dialog.title}
            </DialogTitle>
            <DialogDescription className="sr-only">
              {t.settings.dialog.description}
            </DialogDescription>
            <nav className="flex flex-col gap-[1px]">
              {sections.map((section) => (
                <button
                  key={section.id}
                  type="button"
                  onClick={() => setActive(section.id)}
                  className={cn(
                    "flex items-center gap-2 rounded-[6px] px-3 py-2 text-left text-[12.5px] tracking-[-0.005em] transition-colors",
                    active === section.id
                      ? "bg-card font-medium text-foreground"
                      : "text-muted-foreground hover:bg-[color-mix(in_oklab,var(--foreground)_4%,transparent)] hover:text-foreground",
                  )}
                  style={
                    active === section.id
                      ? { boxShadow: "var(--shadow-sm)" }
                      : undefined
                  }
                >
                  <span className="flex-1">{section.label}</span>
                </button>
              ))}
            </nav>
          </aside>

          <div className="h-full min-h-0 overflow-y-auto bg-background px-8 py-7">
            {active === "general" ? <GeneralSection /> : null}
            {active === "capture" ? <CapturePrivacySection /> : null}
            {active === "quick-ask" ? <QuickAskSection /> : null}
            {active === "exec-agent" ? <ExecAgentSection /> : null}
            {active === "skills" ? <SkillsSection /> : null}
            {active === "memory" ? <MemorySection /> : null}
            {active === "integrations" ? <IntegrationsSection /> : null}
            {active === "shortcuts" ? <ShortcutsSection /> : null}
            {active === "about" ? <AboutSection /> : null}
            {active === "developer" ? <DeveloperSection /> : null}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
