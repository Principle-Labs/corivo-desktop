import { useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";
import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";
import { Button } from "@repo/ui/components/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@repo/ui/components/dialog";
import { cn } from "@repo/ui/lib/utils";
import { useConfig } from "@/hooks/use-config";
import { useTranslation } from "@/i18n";
import type { Language, ThemePreference } from "@/lib/types";
import {
  dataHardDelete,
  fromInvokeError,
  getAutostartEnabled,
  resetOnboarding,
  setAutostartEnabled,
} from "@/lib/tauri";
import {
  FieldGroup,
  SectionHeader,
  SettingsSkeleton,
  ToggleRow,
} from "./settings-shared";

export function GeneralSection() {
  const { t } = useTranslation();
  const { config, isLoading, update } = useConfig();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const autostartQuery = useQuery({
    queryKey: ["autostart"],
    queryFn: getAutostartEnabled,
  });
  const autostartMutation = useMutation({
    mutationFn: setAutostartEnabled,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["autostart"] });
      toast.success(t.settings.general.autostartUpdated);
    },
    onError: (error: unknown) => {
      toast.error(t.common.setupFailed(fromInvokeError(error)));
    },
  });
  const resetOnboardingMutation = useMutation({
    mutationFn: resetOnboarding,
    onSuccess: () => {
      toast.success(t.settings.general.resetOnboarding.success);
      navigate({ to: "/onboarding/permission" });
    },
    onError: (error: unknown) => {
      toast.error(t.common.resetFailed(fromInvokeError(error)));
    },
  });
  const [confirmText, setConfirmText] = useState("");
  const [hardDeleteOpen, setHardDeleteOpen] = useState(false);
  const hardDeleteMutation = useMutation({
    mutationFn: dataHardDelete,
    onSuccess: (summary) => {
      toast.success(
        t.settings.general.hardDelete.success(
          summary.frames_before,
          summary.sessions_deleted,
          summary.screenshots_deleted,
        ),
      );
      setConfirmText("");
      setHardDeleteOpen(false);
      void queryClient.invalidateQueries();
    },
    onError: (error: unknown) => {
      toast.error(t.common.deleteOpFailed(fromInvokeError(error)));
    },
  });

  if (isLoading || !config) {
    return <SettingsSkeleton />;
  }

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.general.title}
        description={t.settings.general.description}
      />

      <FieldGroup title={t.settings.general.startupGroup}>
        <ToggleRow
          label={t.settings.general.autostart.label}
          description={t.settings.general.autostart.description}
          value={autostartQuery.data ?? false}
          onChange={(value) => autostartMutation.mutate(value)}
        />
        <ToggleRow
          label={t.settings.general.autoCapture.label}
          description={t.settings.general.autoCapture.description}
          value={config.app.start_capture_on_launch}
          onChange={(value) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, start_capture_on_launch: value },
            }))
          }
        />
      </FieldGroup>

      <FieldGroup title={t.settings.general.windowGroup}>
        <ToggleRow
          label={t.settings.general.minimizeToTray.label}
          description={t.settings.general.minimizeToTray.description}
          value={config.app.minimize_to_tray}
          onChange={(value) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, minimize_to_tray: value },
            }))
          }
        />
      </FieldGroup>

      <FieldGroup title={t.settings.general.languageGroup}>
        <div className="space-y-2">
          <Label className="text-sm">
            {t.settings.general.uiLanguage.label}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t.settings.general.uiLanguage.description}
          </p>
          <select
            value={config.app.ui_language}
            onChange={(event) =>
              update((prev) => ({
                ...prev,
                app: {
                  ...prev.app,
                  ui_language: event.target.value as Language,
                },
              }))
            }
            className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
          >
            <option value="zh">{t.settings.general.languageOption.zh}</option>
            <option value="en">{t.settings.general.languageOption.en}</option>
          </select>
        </div>
        <div className="space-y-2">
          <Label className="text-sm">
            {t.settings.general.responseLanguage.label}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t.settings.general.responseLanguage.description}
          </p>
          <select
            value={config.app.response_language}
            onChange={(event) =>
              update((prev) => ({
                ...prev,
                app: {
                  ...prev.app,
                  response_language: event.target.value as Language,
                },
              }))
            }
            className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
          >
            <option value="zh">{t.settings.general.languageOption.zh}</option>
            <option value="en">{t.settings.general.languageOption.en}</option>
          </select>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.general.appearanceGroup}>
        <div className="space-y-2">
          <Label className="text-sm">{t.settings.general.theme.label}</Label>
          <p className="text-xs text-muted-foreground">
            {t.settings.general.theme.description}
          </p>
          <div
            role="radiogroup"
            aria-label={t.settings.general.theme.label}
            className="inline-flex rounded-md border border-input bg-background p-0.5"
          >
            {(["light", "dark", "system"] as const).map((option) => {
              const active = config.app.theme === option;
              return (
                <button
                  key={option}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onClick={() =>
                    update((prev) => ({
                      ...prev,
                      app: {
                        ...prev.app,
                        theme: option as ThemePreference,
                      },
                    }))
                  }
                  className={cn(
                    "rounded-sm px-3 py-1.5 text-sm font-medium transition-colors",
                    active
                      ? "bg-foreground text-background shadow-sm"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {t.settings.general.theme.options[option]}
                </button>
              );
            })}
          </div>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.general.onboardingGroup}>
        <div className="flex items-center justify-between gap-4">
          <div className="flex-1 pr-4">
            <Label className="text-sm">
              {t.settings.general.resetOnboarding.label}
            </Label>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {t.settings.general.resetOnboarding.description}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={resetOnboardingMutation.isPending}
            onClick={() => resetOnboardingMutation.mutate()}
          >
            {t.settings.general.resetOnboarding.action}
          </Button>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.general.dataGroup}>
        <div className="flex items-start justify-between gap-4 pb-3">
          <div className="flex-1 space-y-0.5">
            <Label className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.general.retention.label}
            </Label>
            <p className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.general.retention.description}
            </p>
          </div>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.general.hardDeleteGroup}>
        <div className="flex items-center justify-between gap-4">
          <div className="flex-1 space-y-0.5">
            <Label className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.general.hardDelete.label}
            </Label>
            <p className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.general.hardDelete.description}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0 border-destructive/40 text-destructive hover:bg-destructive/5 hover:text-destructive"
            onClick={() => setHardDeleteOpen(true)}
          >
            {t.settings.general.hardDelete.openButton}
          </Button>
        </div>
      </FieldGroup>

      <Dialog
        open={hardDeleteOpen}
        onOpenChange={(next) => {
          if (!next) setConfirmText("");
          setHardDeleteOpen(next);
        }}
      >
        <DialogContent className="sm:max-w-[420px]">
          <DialogHeader>
            <DialogTitle className="font-display text-[17px] font-semibold tracking-[-0.015em] text-destructive">
              {t.settings.general.hardDelete.confirmTitle}
            </DialogTitle>
            <DialogDescription className="text-[13px] leading-[1.55]">
              {t.settings.general.hardDelete.description}
            </DialogDescription>
          </DialogHeader>
          <div className="py-2">
            <Label className="mb-1.5 block text-[12px] font-medium tracking-[-0.005em] text-muted-foreground">
              {t.settings.general.hardDelete.confirmHint}
            </Label>
            <Input
              value={confirmText}
              onChange={(event) => setConfirmText(event.target.value)}
              placeholder="DELETE"
              autoFocus
              disabled={hardDeleteMutation.isPending}
            />
          </div>
          <DialogFooter className="gap-2 sm:gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setHardDeleteOpen(false);
                setConfirmText("");
              }}
              disabled={hardDeleteMutation.isPending}
            >
              {t.common.cancel}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={
                confirmText !== "DELETE" || hardDeleteMutation.isPending
              }
              onClick={() => hardDeleteMutation.mutate()}
            >
              {hardDeleteMutation.isPending
                ? t.settings.general.hardDelete.clearing
                : t.settings.general.hardDelete.clear}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
