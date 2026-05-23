import { useQuery } from "@tanstack/react-query";
import { useShallow } from "zustand/react/shallow";
import { Button } from "@repo/ui/components/button";
import { useCapabilities } from "@/hooks/use-capabilities";
import { useTranslation } from "@/i18n";
import { getSystemInfo } from "@/lib/tauri";
import { useUpdaterStore } from "@/stores/updater-store";
import { FieldGroup, SectionHeader, SettingsSkeleton } from "./settings-shared";

export function AboutSection() {
  const { t } = useTranslation();
  const infoQuery = useQuery({
    queryKey: ["system-info"],
    queryFn: getSystemInfo,
  });
  const { data: capabilities } = useCapabilities();
  const updater = useUpdaterStore(
    useShallow((state) => ({
      status: state.status,
      availableVersion: state.availableVersion,
      error: state.error,
      checkForUpdates: state.checkForUpdates,
      installUpdate: state.installUpdate,
    })),
  );

  if (infoQuery.isLoading) {
    return <SettingsSkeleton />;
  }

  const isBusy =
    updater.status === "checking" ||
    updater.status === "updating" ||
    updater.status === "restarting" ||
    updater.status === "awaiting-restart";
  const hasPendingUpdate =
    updater.availableVersion !== null &&
    (updater.status === "available" ||
      updater.status === "updating" ||
      updater.status === "restarting" ||
      updater.status === "error");
  const actionLabel =
    updater.status === "awaiting-restart"
      ? t.updater.awaitingRestartLabel
      : hasPendingUpdate
        ? updater.status === "updating" || updater.status === "restarting"
          ? t.settings.about.updating
          : t.settings.about.doUpdate
        : updater.status === "checking"
          ? t.settings.about.checking
          : t.settings.about.checkUpdate;
  const canUseUpdater = capabilities?.managedUpdater === true;

  async function handleUpdateAction() {
    if (hasPendingUpdate) {
      await updater.installUpdate();
      return;
    }

    await updater.checkForUpdates();
  }

  // updaterStore stores `null` for "no specific message" — fall back to
  // the localized default so the displayed copy follows ui_language.
  // awaiting-restart 是"装好但没重启"，用专门的提示文案而不是失败文案。
  const updaterError =
    updater.status === "awaiting-restart"
      ? t.updater.awaitingRestart
      : updater.status === "error"
        ? (updater.error ?? t.updater.failedFallback)
        : null;

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.about.title}
        description={t.settings.about.description}
      />

      <FieldGroup title={t.settings.about.sysInfoGroup}>
        <div className="space-y-3 text-sm">
          <div className="flex items-center gap-3">
            <div>
              {t.settings.about.versionLabel(
                infoQuery.data?.app_version ?? "",
              )}
            </div>
            {canUseUpdater ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={isBusy}
                onClick={() => {
                  void handleUpdateAction();
                }}
              >
                {actionLabel}
              </Button>
            ) : null}
          </div>
          {canUseUpdater && updater.availableVersion ? (
            <div className="text-xs text-muted-foreground">
              {t.settings.about.newVersionLabel(updater.availableVersion)}
            </div>
          ) : null}
          {canUseUpdater && updaterError ? (
            <div className="text-xs text-destructive">{updaterError}</div>
          ) : null}
          <div>{t.settings.about.osLabel(infoQuery.data?.os ?? "")}</div>
          <div>
            {t.settings.about.osVersionLabel(infoQuery.data?.os_version ?? "")}
          </div>
          <div>{t.settings.about.archLabel(infoQuery.data?.arch ?? "")}</div>
          <div>
            {t.settings.about.tauriLabel(infoQuery.data?.tauri_version ?? "")}
          </div>
        </div>
      </FieldGroup>

      <FieldGroup title={t.settings.about.linksGroup}>
        <div className="flex flex-wrap gap-2">
          <Button asChild variant="outline">
            <a
              href="https://corivo.ai/changelog"
              target="_blank"
              rel="noreferrer"
            >
              {t.settings.about.releaseNotes}
            </a>
          </Button>
          <Button asChild variant="outline">
            <a href="mailto:hi@corivo.ai">
              {t.settings.about.feedback}
            </a>
          </Button>
        </div>
      </FieldGroup>
    </div>
  );
}
