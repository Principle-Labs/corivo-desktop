import { useNavigate } from "@tanstack/react-router";
import { useMutation } from "@tanstack/react-query";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { toast } from "sonner";
import { Button } from "@repo/ui/components/button";
import { useConfig } from "@/hooks/use-config";
import { useTranslation } from "@/i18n";
import {
  devOpenDevtools,
  devRevealDataDir,
  resetOnboarding,
  type DevWindowLabel,
} from "@/lib/tauri";
import type { Config } from "@/lib/types";
import {
  FieldGroup,
  SectionHeader,
  SettingsSkeleton,
  ToggleRow,
} from "./settings-shared";

/// Redact secret fields before the user copies the Config JSON for a
/// bug report. Keeps the structure identical so the receiver can still
/// see which slot was populated — just with the value masked. The
/// fields targeted are: BYOK API key (Settings → Execution engine) and
/// the Corivo closed-beta bearer token.
function redactConfig(config: Config): Config {
  return {
    ...config,
    exec_agent: {
      ...config.exec_agent,
      byok_key: config.exec_agent.byok_key ? "***REDACTED***" : null,
    },
    corivo_session: {
      ...config.corivo_session,
      session_token: config.corivo_session.session_token
        ? "***REDACTED***"
        : null,
    },
  };
}

export function DeveloperSection() {
  const { t } = useTranslation();
  const { config, isLoading, update } = useConfig();
  const navigate = useNavigate();

  const openDevtoolsMutation = useMutation({
    mutationFn: async (args: { label: DevWindowLabel; displayName: string }) => {
      await devOpenDevtools(args.label);
      return args.displayName;
    },
    onSuccess: (displayName) => {
      toast.success(t.settings.developer.devtoolsOpened(displayName));
    },
    onError: (error: unknown) => {
      toast.error(t.settings.developer.devtoolsFailed(String(error)));
    },
  });

  const revealDataDirMutation = useMutation({
    mutationFn: devRevealDataDir,
    onError: (error: unknown) => {
      toast.error(t.settings.developer.revealDataDir.failed(String(error)));
    },
  });

  const resetOnboardingMutation = useMutation({
    mutationFn: resetOnboarding,
    onSuccess: () => {
      toast.success(t.settings.developer.resetOnboarding.success);
      navigate({ to: "/onboarding/permission" });
    },
    onError: (error: unknown) => {
      toast.error(t.settings.developer.resetOnboarding.failed(String(error)));
    },
  });

  if (isLoading || !config) {
    return <SettingsSkeleton />;
  }

  async function handleCopyConfig() {
    if (!config) return;
    try {
      const payload = JSON.stringify(redactConfig(config), null, 2);
      await writeText(payload);
      toast.success(t.settings.developer.copyConfig.success);
    } catch (error) {
      toast.error(t.settings.developer.copyConfig.failed(String(error)));
    }
  }

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.developer.title}
        description={t.settings.developer.description}
      />

      <FieldGroup title={t.settings.developer.modeGroup}>
        <ToggleRow
          label={t.settings.developer.modeToggle.label}
          description={t.settings.developer.modeToggle.description}
          value={config.app.developer_mode}
          onChange={(value) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, developer_mode: value },
            }))
          }
        />
      </FieldGroup>

      <FieldGroup title={t.settings.developer.devtoolsGroup}>
        <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
          {t.settings.developer.devtoolsDescription}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={openDevtoolsMutation.isPending}
            onClick={() =>
              openDevtoolsMutation.mutate({
                label: "main",
                displayName: t.settings.developer.devtoolsMain,
              })
            }
          >
            {t.settings.developer.devtoolsMain}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={openDevtoolsMutation.isPending}
            onClick={() =>
              openDevtoolsMutation.mutate({
                label: "quick-ask",
                displayName: t.settings.developer.devtoolsQuickAsk,
              })
            }
          >
            {t.settings.developer.devtoolsQuickAsk}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={openDevtoolsMutation.isPending}
            onClick={() =>
              openDevtoolsMutation.mutate({
                label: "notification-overlay",
                displayName: t.settings.developer.devtoolsOverlay,
              })
            }
          >
            {t.settings.developer.devtoolsOverlay}
          </Button>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.developer.dataGroup}>
        <div className="flex items-center justify-between gap-4 border-b border-border pb-3">
          <div className="flex-1 space-y-0.5">
            <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.developer.revealDataDir.label}
            </div>
            <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.developer.revealDataDir.description}
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={revealDataDirMutation.isPending}
            onClick={() => revealDataDirMutation.mutate()}
          >
            {t.settings.developer.revealDataDir.action}
          </Button>
        </div>
        <div className="flex items-center justify-between gap-4 pb-1">
          <div className="flex-1 space-y-0.5">
            <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.developer.copyConfig.label}
            </div>
            <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.developer.copyConfig.description}
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              void handleCopyConfig();
            }}
          >
            {t.settings.developer.copyConfig.action}
          </Button>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.developer.runtimeGroup}>
        <div className="flex items-center justify-between gap-4 border-b border-border pb-3">
          <div className="flex-1 space-y-0.5">
            <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.developer.resetOnboarding.label}
            </div>
            <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.developer.resetOnboarding.description}
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={resetOnboardingMutation.isPending}
            onClick={() => resetOnboardingMutation.mutate()}
          >
            {t.settings.developer.resetOnboarding.action}
          </Button>
        </div>
        <div className="flex items-center justify-between gap-4 pb-1">
          <div className="flex-1 space-y-0.5">
            <div className="text-[13px] font-medium tracking-[-0.005em] text-foreground">
              {t.settings.developer.reloadWebview.label}
            </div>
            <div className="text-[11.5px] leading-[1.5] text-muted-foreground">
              {t.settings.developer.reloadWebview.description}
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              window.location.reload();
            }}
          >
            {t.settings.developer.reloadWebview.action}
          </Button>
        </div>
      </FieldGroup>
    </div>
  );
}
