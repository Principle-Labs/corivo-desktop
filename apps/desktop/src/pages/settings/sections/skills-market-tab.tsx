// "市场" tab —— 浏览 Corivo skill 市场目录、一键安装 / 卸载。
//
// 安装实际写到 ~/.corivo/skills/market/<slug>/，由 services::skill_share
// 的 scan() 兜底识别为 SkillSource::Market；启用 (= "本地" tab 勾选) 后
// 走 symlink 同步进 $APPDATA/claude-config/skills/，agent 自动看到。
//
// 数据：
//   - marketList(authToken?) ── 远端目录 (按身份过滤，匿名只看 public)
//   - marketInstalled()      ── 本地 .market-meta.json，用于状态对齐
//                                (已安装 / 可升级判定)
// 当前 P1：未登录态调用，authToken=undefined。closed overlay 在挂上
// session 之后可以传入 token 让登录的 internal 用户多看 internal skill。

import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Loader2, RefreshCcw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";

import { useTranslation } from "@/i18n";
import {
  marketInstall,
  marketInstalled,
  marketList,
  marketUninstall,
  type MarketSkill,
  type MarketMeta,
} from "@/lib/market";
import { fromInvokeError } from "@/lib/tauri";

const MARKET_LIST_KEY = ["market-list"] as const;
const MARKET_INSTALLED_KEY = ["market-installed"] as const;
const SKILLS_AVAILABLE_KEY = ["skills-available"] as const;

type RowState =
  | { kind: "not_installed" }
  | { kind: "installed"; meta: MarketMeta }
  | { kind: "update_available"; meta: MarketMeta };

function computeRowState(
  skill: MarketSkill,
  installed: MarketMeta[],
): RowState {
  const meta = installed.find((m) => m.slug === skill.slug);
  if (!meta) return { kind: "not_installed" };
  if (meta.commit_sha !== skill.commit_sha) {
    return { kind: "update_available", meta };
  }
  return { kind: "installed", meta };
}

export function SkillsMarketTab() {
  const { t } = useTranslation();
  const qc = useQueryClient();

  const listQuery = useQuery({
    queryKey: MARKET_LIST_KEY,
    queryFn: () => marketList(),
    refetchOnWindowFocus: false,
    // 远端目录变化频率低，stale 30s 已经够了。
    staleTime: 30_000,
  });
  const installedQuery = useQuery({
    queryKey: MARKET_INSTALLED_KEY,
    queryFn: marketInstalled,
    refetchOnWindowFocus: false,
  });

  const installMut = useMutation({
    mutationFn: (slug: string) => marketInstall(slug),
    onSuccess: (_meta, slug) => {
      toast.success(t.settings.skills.market.installSuccess(slug));
      // 本地 .market-meta.json 列表变了
      void qc.invalidateQueries({ queryKey: MARKET_INSTALLED_KEY });
      // 本地 skill 扫描列表也会多一条 (SkillSource::Market)
      void qc.invalidateQueries({ queryKey: SKILLS_AVAILABLE_KEY });
    },
    onError: (err, slug) => {
      toast.error(
        t.settings.skills.market.installFailed(slug, fromInvokeError(err)),
      );
    },
  });

  const uninstallMut = useMutation({
    mutationFn: (slug: string) => marketUninstall(slug),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MARKET_INSTALLED_KEY });
      void qc.invalidateQueries({ queryKey: SKILLS_AVAILABLE_KEY });
    },
    onError: (err, slug) => {
      toast.error(
        t.settings.skills.market.uninstallFailed(slug, fromInvokeError(err)),
      );
    },
  });

  const skills = listQuery.data ?? [];
  const installed = installedQuery.data ?? [];

  const rows = useMemo(
    () =>
      skills.map((skill) => ({
        skill,
        state: computeRowState(skill, installed),
      })),
    [skills, installed],
  );

  const refreshing = listQuery.isFetching || installedQuery.isFetching;

  return (
    <div className="space-y-4">
      <p className="text-[11.5px] leading-[1.55] text-muted-foreground">
        {t.settings.skills.market.description}
      </p>

      <div className="flex items-center justify-end">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            void listQuery.refetch();
            void installedQuery.refetch();
          }}
          disabled={refreshing}
        >
          <RefreshCcw className="mr-1 h-3 w-3" />
          {t.settings.skills.market.refresh}
        </Button>
      </div>

      {listQuery.isLoading ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          {t.settings.skills.market.loading}
        </div>
      ) : listQuery.isError ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-xs text-destructive">
          {t.settings.skills.market.loadFailed}
        </div>
      ) : rows.length === 0 ? (
        <div className="rounded-md border border-dashed border-border/60 p-4 text-xs text-muted-foreground">
          {t.settings.skills.market.empty}
        </div>
      ) : (
        <ul className="divide-y divide-border/40 rounded-md border border-border/40">
          {rows.map(({ skill, state }) => {
            const installing =
              installMut.isPending && installMut.variables === skill.slug;
            const uninstalling =
              uninstallMut.isPending && uninstallMut.variables === skill.slug;
            return (
              <li key={skill.slug} className="p-3">
                <div className="flex items-start gap-3">
                  <div className="min-w-0 flex-1 space-y-1.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-sm font-medium">
                        {skill.name || skill.slug}
                      </span>
                      {skill.visibility === "internal" ? (
                        <span className="rounded-sm bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-400">
                          {t.settings.skills.market.visibilityInternal}
                        </span>
                      ) : null}
                      {skill.has_scripts ? (
                        <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                          {t.settings.skills.market.hasScriptsBadge}
                        </span>
                      ) : null}
                    </div>
                    {skill.description ? (
                      <div className="text-xs leading-[1.5] text-muted-foreground">
                        {skill.description}
                      </div>
                    ) : null}
                    {skill.tags.length > 0 ? (
                      <div className="flex flex-wrap gap-1">
                        {skill.tags.map((tag) => (
                          <span
                            key={tag}
                            className="rounded-sm bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                          >
                            {tag}
                          </span>
                        ))}
                      </div>
                    ) : null}
                  </div>
                  <RowAction
                    state={state}
                    installing={installing}
                    uninstalling={uninstalling}
                    onInstall={() => installMut.mutate(skill.slug)}
                    onUninstall={() => uninstallMut.mutate(skill.slug)}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function RowAction({
  state,
  installing,
  uninstalling,
  onInstall,
  onUninstall,
}: {
  state: RowState;
  installing: boolean;
  uninstalling: boolean;
  onInstall: () => void;
  onUninstall: () => void;
}) {
  const { t } = useTranslation();
  if (state.kind === "not_installed") {
    return (
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={onInstall}
        disabled={installing}
      >
        {installing ? (
          <Loader2 className="mr-1 h-3 w-3 animate-spin" />
        ) : (
          <Download className="mr-1 h-3 w-3" />
        )}
        {installing
          ? t.settings.skills.market.installing
          : t.settings.skills.market.install}
      </Button>
    );
  }
  // installed 或 update_available 都给 [升级|已安装] + [卸载] 两个按钮
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      {state.kind === "update_available" ? (
        <Button
          type="button"
          size="sm"
          variant="default"
          onClick={onInstall}
          disabled={installing}
        >
          {installing ? (
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          ) : null}
          {t.settings.skills.market.updateAvailable}
        </Button>
      ) : (
        <span className="rounded-sm bg-emerald-500/15 px-2 py-0.5 text-[10.5px] font-medium text-emerald-700 dark:text-emerald-400">
          {t.settings.skills.market.installed}
        </span>
      )}
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={onUninstall}
        disabled={uninstalling}
        aria-label={t.settings.skills.market.uninstall}
      >
        {uninstalling ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : (
          <Trash2 className="h-3 w-3" />
        )}
      </Button>
    </div>
  );
}
