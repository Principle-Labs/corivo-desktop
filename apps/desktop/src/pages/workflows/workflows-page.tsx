import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Settings2, Workflow } from "lucide-react";

import { Button } from "@repo/ui/components/button";

import { useTranslation } from "@/i18n";
import { skillsListAvailable } from "@/lib/tauri";
import { openSettingsDialog } from "@/stores/settings-dialog-store";

/**
 * "我的工作流" route page.
 *
 * Surface intent: this page is for **user-facing workflows** —
 * agent-crystalized procedures and user-authored routines. External
 * primitives (lark / Claude / MCP capabilities) are intentionally
 * absent here; they live under Settings → 技能.
 *
 * Classification is purely by source directory on the Rust side
 * (`SkillSource → SkillKind`). We don't touch external SKILL.md files;
 * we just filter the scanner output by `kind === "workflow"`.
 *
 * v1: workflows list is empty because no writable source directory is
 * mounted yet. A footer chip surfaces the count of available
 * capabilities so the user knows the agent has tools available, and
 * links into Settings for the toggle UI.
 */
export function WorkflowsPage() {
  const { t } = useTranslation();
  const { data: skills } = useQuery({
    queryKey: ["skills-available"],
    queryFn: skillsListAvailable,
    refetchOnWindowFocus: false,
  });

  const { workflows, capabilityCount } = useMemo(() => {
    const all = skills ?? [];
    return {
      workflows: all.filter((s) => s.kind === "workflow"),
      capabilityCount: all.filter((s) => s.kind === "capability").length,
    };
  }, [skills]);

  return (
    <div className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-muted-foreground">
          <Workflow className="h-4 w-4" />
          <span className="font-mono text-[11px] uppercase tracking-[0.12em]">
            {t.workflows.eyebrow}
          </span>
        </div>
        <h1 className="font-display text-[24px] font-semibold tracking-[-0.015em] text-foreground">
          {t.workflows.title}
        </h1>
        <p className="max-w-2xl text-[13.5px] leading-[1.6] text-muted-foreground">
          {t.workflows.description}
        </p>
      </header>

      {workflows.length === 0 ? (
        <EmptyState />
      ) : (
        <ul className="divide-y divide-border/40 rounded-lg border border-border/40">
          {workflows.map((skill) => (
            <li
              key={skill.path}
              className="flex flex-col gap-1 p-4 transition-colors hover:bg-muted/30"
            >
              <div className="flex items-center gap-2">
                <span className="font-mono text-[13px] font-medium text-foreground">
                  {skill.name}
                </span>
              </div>
              {skill.description ? (
                <p className="line-clamp-2 text-[12.5px] text-muted-foreground">
                  {skill.description}
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      <CapabilityFooter count={capabilityCount} />
    </div>
  );
}

function EmptyState() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-start gap-3 rounded-lg border border-dashed border-border/60 bg-muted/20 p-6">
      <Workflow className="h-5 w-5 text-muted-foreground/70" />
      <div className="space-y-1.5">
        <p className="font-display text-[15px] font-medium tracking-[-0.005em] text-foreground">
          {t.workflows.empty.title}
        </p>
        <p className="max-w-xl text-[12.5px] leading-[1.65] text-muted-foreground">
          {t.workflows.empty.body}
        </p>
      </div>
    </div>
  );
}

function CapabilityFooter({ count }: { count: number }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/40 bg-card/50 px-4 py-3 text-[12.5px] text-muted-foreground">
      <div className="flex items-center gap-2">
        <Settings2 className="h-3.5 w-3.5 shrink-0" />
        <span>{t.workflows.capabilityFooter.summary(count)}</span>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => openSettingsDialog()}
      >
        {t.workflows.capabilityFooter.manageAction}
      </Button>
    </div>
  );
}
