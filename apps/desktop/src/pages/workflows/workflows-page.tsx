import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { Workflow } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@repo/ui/components/dialog";

import { useWorkflowRuns } from "@/hooks/use-workflows";
import { useTranslation } from "@/i18n";
import {
  fromInvokeError,
  workflowsDelete,
  workflowsList,
  workflowsRunNow,
  workflowsSetEnabled,
} from "@/lib/tauri";
import { useActiveThreadStore } from "@/stores/active-thread-store";
import { WorkflowDrawer } from "@/pages/workflows/workflow-drawer";
import { WorkflowListItem } from "@/pages/workflows/workflow-list-item";

/**
 * "我的工作流" route page (v1430).
 *
 * Lists scheduled workflows + handles the create/edit drawer + run-now
 * mutation + history dialog. Definitions live as
 * `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md`; this page is the
 * authoring + status surface over the backing `workflow_schedules` /
 * `workflow_runs` SQLite tables.
 */
export function WorkflowsPage() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const router = useRouter();
  const selectWorkflowRun = useActiveThreadStore((s) => s.selectWorkflowRun);

  const { data: workflows } = useQuery({
    queryKey: ["workflows-list"],
    queryFn: workflowsList,
    refetchOnWindowFocus: false,
  });
  // Reuse the sidebar's run query so "查看历史" → /ask doesn't pay a
  // second roundtrip for data already in cache. Default limit (50)
  // covers the practical "latest run for this slug" lookup; older
  // runs still surface via the sidebar scroll.
  const { data: workflowRuns } = useWorkflowRuns();

  const [drawerSlug, setDrawerSlug] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  // Pending-delete confirmation dialog. We can't use window.confirm()
  // — Tauri 2's webview silently drops synchronous modals (would
  // block the JS thread + the embedded webview's UI thread), so the
  // delete button looked broken. Render a real Radix Dialog instead.
  const [deletingSlug, setDeletingSlug] = useState<string | null>(null);

  const setEnabled = useMutation({
    mutationFn: ({ slug, enabled }: { slug: string; enabled: boolean }) =>
      workflowsSetEnabled(slug, enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["workflows-list"] }),
    onError: (error) => toast.error(fromInvokeError(error)),
  });

  // "立即运行" — just enqueue. The button briefly shows pending state
  // via `runNow.isPending` (the IPC enqueue is fast); the actual
  // execution result surfaces later via the notification-overlay
  // toast. No in-flight UI on the row.
  const runNow = useMutation({
    mutationFn: workflowsRunNow,
    onError: (error) => toast.error(fromInvokeError(error)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["workflows-list"] });
    },
  });

  const remove = useMutation({
    mutationFn: workflowsDelete,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["workflows-list"] }),
    onError: (error) => toast.error(fromInvokeError(error)),
  });

  const editing = useMemo(
    () => workflows?.find((w) => w.definition.slug === drawerSlug) ?? null,
    [workflows, drawerSlug],
  );

  const list = workflows ?? [];

  /**
   * "查看历史" — find the most-recent run for this slug, open it in
   * the chat viewer (read-only mode), and navigate to `/ask`. The
   * sidebar's unified thread list will also surface every other run
   * for the same slug, so the user can browse historical runs from
   * there. When the workflow has never run, we still navigate to
   * `/ask` and pop a toast so the user understands why nothing
   * loaded — beats silently opening an empty viewer.
   *
   * Replaces the prior `WorkflowHistoryDialog` modal which was a
   * standalone surface duplicating concepts that already live in
   * `/ask` (chat thread viewer + sidebar list).
   */
  const handleShowHistory = (slug: string, workflowName: string) => {
    const runs = workflowRuns ?? [];
    // workflowRuns is pre-sorted started_at DESC by the backend; the
    // first match for this slug is the latest run.
    const latest = runs.find((r) => r.slug === slug && r.thread_id !== null);
    if (!latest || !latest.thread_id) {
      toast.message(t.workflows.history.empty);
      void router.navigate({ to: "/ask" });
      return;
    }
    selectWorkflowRun(latest.thread_id, {
      kind: "workflow_run",
      workflowName,
      slug,
      runStatus: latest.status,
      startedAt: latest.started_at,
      finishedAt: latest.finished_at,
    });
    void router.navigate({ to: "/ask" });
  };

  return (
    <div className="flex flex-col gap-8">
      <header className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-muted-foreground">
          <Workflow className="h-4 w-4" />
          <span className="font-mono text-[11px] uppercase tracking-[0.12em]">
            {t.workflows.eyebrow}
          </span>
        </div>
        <div className="flex items-baseline justify-between gap-4">
          <h1 className="font-display text-[24px] font-semibold tracking-[-0.015em] text-foreground">
            {t.workflows.title}
          </h1>
          <Button
            type="button"
            size="sm"
            onClick={() => {
              setDrawerSlug(null);
              setDrawerOpen(true);
            }}
          >
            {t.workflows.list.newAction}
          </Button>
        </div>
        <p className="max-w-2xl text-[13.5px] leading-[1.6] text-muted-foreground">
          {t.workflows.description}
        </p>
      </header>

      {list.length === 0 ? (
        <EmptyState />
      ) : (
        <ul className="flex flex-col gap-2">
          {list.map((view) => (
            <WorkflowListItem
              key={view.definition.slug}
              view={view}
              onToggle={(enabled) =>
                setEnabled.mutate({
                  slug: view.definition.slug,
                  enabled,
                })
              }
              onRunNow={() => runNow.mutate(view.definition.slug)}
              onEdit={() => {
                setDrawerSlug(view.definition.slug);
                setDrawerOpen(true);
              }}
              onShowHistory={() => handleShowHistory(view.definition.slug, view.definition.name)}
              onDelete={() => setDeletingSlug(view.definition.slug)}
            />
          ))}
        </ul>
      )}

      <WorkflowDrawer
        open={drawerOpen}
        onOpenChange={(open) => {
          setDrawerOpen(open);
          if (!open) setDrawerSlug(null);
        }}
        editing={editing}
        existingSlugs={list.map((w) => w.definition.slug)}
      />

      <Dialog
        open={deletingSlug !== null}
        onOpenChange={(open) => {
          if (!open) setDeletingSlug(null);
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              {t.workflows.list.deleteConfirmTitle}
            </DialogTitle>
            <DialogDescription>
              {deletingSlug
                ? t.workflows.list.deleteConfirm(
                    list.find((w) => w.definition.slug === deletingSlug)
                      ?.definition.name ?? deletingSlug,
                  )
                : null}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDeletingSlug(null)}
            >
              {t.workflows.drawer.cancelAction}
            </Button>
            <Button
              type="button"
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                if (deletingSlug) {
                  remove.mutate(deletingSlug);
                  setDeletingSlug(null);
                }
              }}
            >
              {t.workflows.list.delete}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
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
