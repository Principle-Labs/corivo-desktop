import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Loader2, RefreshCcw } from "lucide-react";

import { Button } from "@repo/ui/components/button";

import { useConfig } from "@/hooks/use-config";
import { useTranslation } from "@/i18n";
import { skillsListAvailable } from "@/lib/tauri";
import type { AvailableSkill } from "@/lib/types";

import {
  FieldGroup,
  SectionHeader,
  SettingsSkeleton,
} from "./settings-shared";

const SOURCE_LABEL: Record<AvailableSkill["source"], string> = {
  agents: "~/.agents/skills/",
  claude: "~/.claude/skills/",
};

export function SkillsSection() {
  const { t } = useTranslation();
  const { config, isLoading, update, isSaving } = useConfig();
  const {
    data: available,
    isLoading: scanning,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: ["skills-available"],
    queryFn: skillsListAvailable,
    refetchOnWindowFocus: false,
  });

  const enabled = useMemo(
    () => new Set(config?.exec_agent.skill_share.enabled ?? []),
    [config],
  );

  if (isLoading || !config) {
    return <SettingsSkeleton />;
  }

  const setEnabled = (next: string[]) => {
    update((prev) => ({
      ...prev,
      exec_agent: {
        ...prev.exec_agent,
        skill_share: { ...prev.exec_agent.skill_share, enabled: next },
      },
    }));
  };

  const toggle = (name: string) => {
    const next = new Set(enabled);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    setEnabled(Array.from(next).sort());
  };

  const selectAll = () => {
    if (!available) return;
    setEnabled(available.map((s) => s.name).sort());
  };

  const clearAll = () => setEnabled([]);

  const onlyCorivoProxy = config.exec_agent.auth_mode === "corivo_proxy";

  return (
    <div className="max-w-2xl space-y-6">
      <SectionHeader
        title={t.settings.skills.title}
        description={t.settings.skills.description}
      />

      {!onlyCorivoProxy ? (
        <div className="rounded-md border border-border/40 bg-muted/40 p-3 text-xs text-muted-foreground">
          {t.settings.skills.onlyCorivoMode}
        </div>
      ) : null}

      <FieldGroup title={t.settings.skills.availableTitle}>
        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>
            {t.settings.skills.checkedSummary(
              enabled.size,
              available?.length ?? 0,
            )}
          </span>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => void refetch()}
              disabled={isFetching}
            >
              <RefreshCcw className="mr-1 h-3 w-3" />
              {t.common.refresh}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={selectAll}
              disabled={!available || available.length === 0 || isSaving}
            >
              {t.common.selectAll}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={clearAll}
              disabled={enabled.size === 0 || isSaving}
            >
              {t.common.clearAll}
            </Button>
          </div>
        </div>

        {scanning ? (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            {t.settings.skills.scanning}
          </div>
        ) : !available || available.length === 0 ? (
          <div className="rounded-md border border-dashed border-border/60 p-4 text-xs text-muted-foreground">
            {t.settings.skills.empty}
          </div>
        ) : (
          <ul className="divide-y divide-border/40 rounded-md border border-border/40">
            {available.map((skill) => {
              const checked = enabled.has(skill.name);
              return (
                <li key={skill.name}>
                  <label className="flex cursor-pointer items-start gap-3 p-3 hover:bg-accent/30">
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={isSaving}
                      onChange={() => toggle(skill.name)}
                      className="mt-1 h-4 w-4"
                    />
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-sm font-medium">
                          {skill.name}
                        </span>
                        <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                          {SOURCE_LABEL[skill.source]}
                        </span>
                      </div>
                      {skill.description ? (
                        <div className="line-clamp-2 text-xs text-muted-foreground">
                          {skill.description}
                        </div>
                      ) : null}
                      <div
                        className="truncate font-mono text-[10px] text-muted-foreground/80"
                        title={skill.path}
                      >
                        {skill.path}
                      </div>
                    </div>
                  </label>
                </li>
              );
            })}
          </ul>
        )}

        <p className="text-[11px] text-muted-foreground">
          {t.settings.skills.footnote}
        </p>
      </FieldGroup>
    </div>
  );
}

