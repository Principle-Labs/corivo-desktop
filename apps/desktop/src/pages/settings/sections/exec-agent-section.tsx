import { useEffect, useMemo } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@repo/ui/components/button";
import { Input } from "@repo/ui/components/input";
import { Label } from "@repo/ui/components/label";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@repo/ui/components/tabs";

import { useCapabilities } from "@/hooks/use-capabilities";
import { useConfig } from "@/hooks/use-config";
import { useTranslation } from "@/i18n";
import {
  authLogout,
  authStatus,
  chatgptAuthLogin,
  chatgptAuthLogout,
  chatgptAuthStatus,
  fromInvokeError,
  modelsGetAvailable,
  modelsRefresh,
  type ModelMeta,
} from "@/lib/tauri";
import type { ModelDirectory } from "@corivo/shared-types";
import type {
  ApiShape,
  ExecAgentAuthMode,
  ThinkingLevel,
} from "@/lib/types";
import { applyAuthStatus } from "@/stores/user-profile-store";

import {
  FieldGroup,
  SectionHeader,
  SettingsSkeleton,
} from "./settings-shared";

const THINKING_LEVELS: { value: ThinkingLevel }[] = [
  { value: "off" },
  { value: "minimal" },
  { value: "low" },
  { value: "medium" },
  { value: "high" },
  { value: "xhigh" },
];

const API_SHAPES: { value: ApiShape; label: string; placeholder: string }[] = [
  {
    value: "anthropic",
    label: "Anthropic Messages",
    placeholder: "https://api.anthropic.com",
  },
  {
    value: "openai",
    label: "OpenAI / OpenAI-compatible",
    placeholder: "https://api.openai.com/v1",
  },
];

export function ExecAgentSection() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { config, isLoading, update, isSaving } = useConfig();
  const { data: capabilities } = useCapabilities();
  const cloudAuthAvailable = capabilities?.auth ?? false;
  const cloudModelsAvailable = capabilities?.modelsDirectory ?? false;
  const queryClient = useQueryClient();
  const { data: auth } = useQuery({
    queryKey: ["auth-status"],
    queryFn: authStatus,
    enabled: cloudAuthAvailable,
    refetchOnWindowFocus: false,
  });
  const { data: chatgptAuth, refetch: refetchChatgpt } = useQuery({
    queryKey: ["chatgpt-auth-status"],
    queryFn: chatgptAuthStatus,
    refetchOnWindowFocus: false,
  });

  const chatgptLogin = useMutation({
    mutationFn: chatgptAuthLogin,
    onSuccess: () => {
      toast.success(t.settings.execAgent.chatgptLoginSuccess);
      void refetchChatgpt();
    },
    onError: (error) =>
      toast.error(
        t.settings.execAgent.chatgptLoginFailed(fromInvokeError(error)),
      ),
  });

  const chatgptLogout = useMutation({
    mutationFn: chatgptAuthLogout,
    onSuccess: () => {
      toast.success(t.settings.execAgent.chatgptLogoutSuccess);
      void refetchChatgpt();
    },
    onError: (error) => toast.error(t.common.logoutFailed(fromInvokeError(error))),
  });
  const { data: directory, isFetching: modelsLoading } =
    useQuery<ModelDirectory>({
      queryKey: ["models-available"],
      queryFn: modelsGetAvailable,
      enabled: cloudModelsAvailable,
      refetchOnWindowFocus: false,
    });

  const refreshModels = useMutation({
    mutationFn: modelsRefresh,
    onSuccess: (next) => {
      queryClient.setQueryData<ModelDirectory>(["models-available"], next);
      toast.success(t.settings.execAgent.modelsRefreshed);
    },
    onError: (error) =>
      toast.error(t.settings.execAgent.refreshFailed(fromInvokeError(error))),
  });

  const logout = useMutation({
    mutationFn: authLogout,
    onSuccess: () => {
      // Wipe the sidebar profile cache before navigating; otherwise
      // the UserCard would briefly flash the old name+avatar on the
      // /login splash.
      applyAuthStatus(null);
      toast.success(t.settings.execAgent.logoutSuccess);
      void queryClient.invalidateQueries({ queryKey: ["auth-status"] });
      if (cloudAuthAvailable) {
        void navigate({ to: "/login", replace: true });
      }
    },
    onError: (error) =>
      toast.error(t.common.logoutFailed(fromInvokeError(error))),
  });

  // Every alias the user has been granted. The directory comes from
  // /v1/me/models — no client-side filtering (the backend already
  // dropped disabled rows). Empty array while the cache is hydrating
  // or when the user has no grants.
  const managed = useMemo<ModelMeta[]>(
    () => directory?.managed ?? [],
    [directory],
  );

  // Pin `selected_model_id` to an alias the live directory still has.
  // Covers (a) first launch before the cache lands, (b) admin revoking
  // the previously-selected alias, (c) old config rows that held a
  // raw upstream id from before the alias migration — those won't
  // match any current alias, so we fall back to the backend's
  // default_alias. BYOK skipped because that path drives the sidecar
  // via `byok_model`.
  useEffect(() => {
    if (!config || isSaving) return;
    if (config.exec_agent.auth_mode !== "corivo_proxy") return;
    if (managed.length === 0) return;
    const current = config.exec_agent.selected_model_id;
    if (current && managed.some((m) => m.alias === current)) return;
    const fallback =
      managed.find((m) => m.alias === directory?.default_alias)?.alias ??
      managed[0].alias;
    update((prev) => ({
      ...prev,
      exec_agent: { ...prev.exec_agent, selected_model_id: fallback },
    }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config, managed, directory?.default_alias, isSaving]);

  if (isLoading || !config) {
    return <SettingsSkeleton />;
  }

  const authMode = config.exec_agent.auth_mode;
  const selectedModelId = config.exec_agent.selected_model_id;
  const thinkingLevel = config.exec_agent.thinking_level;
  const byokApiShape = config.exec_agent.byok_api_shape ?? "anthropic";
  const byokBaseUrl = config.exec_agent.byok_base_url ?? "";
  const byokKey = config.exec_agent.byok_key ?? "";
  const byokModel = config.exec_agent.byok_model ?? "";

  const setAuthMode = (next: ExecAgentAuthMode) => {
    if (next === authMode) return;
    update((prev) => ({
      ...prev,
      exec_agent: { ...prev.exec_agent, auth_mode: next },
    }));
  };

  const setSelectedModelId = (next: string) => {
    if (next === (selectedModelId ?? "")) return;
    update((prev) => ({
      ...prev,
      exec_agent: { ...prev.exec_agent, selected_model_id: next || null },
    }));
  };

  const setThinkingLevel = (next: ThinkingLevel) => {
    if (next === thinkingLevel) return;
    update((prev) => ({
      ...prev,
      exec_agent: { ...prev.exec_agent, thinking_level: next },
    }));
  };

  const setByokField = (
    field: "byok_api_shape" | "byok_base_url" | "byok_key" | "byok_model",
    next: string,
  ) => {
    update((prev) => ({
      ...prev,
      exec_agent: {
        ...prev.exec_agent,
        [field]:
          field === "byok_api_shape"
            ? (next as ApiShape)
            : next.length === 0
              ? null
              : next,
      },
    }));
  };

  // When the build doesn't ship cloud auth, the Corivo tab is disabled.
  // If config still says "corivo_proxy", surface BYOK as the active tab
  // so the user sees usable content. We don't rewrite config here —
  // that happens when they actually click a different tab.
  const activeTab: ExecAgentAuthMode =
    !cloudAuthAvailable && authMode === "corivo_proxy" ? "byok" : authMode;

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title={t.settings.execAgent.title}
        description={t.settings.execAgent.description}
      />

      <Tabs
        value={activeTab}
        onValueChange={(next) => setAuthMode(next as ExecAgentAuthMode)}
        className="space-y-6"
      >
        <TabsList className="grid w-full grid-cols-3">
          <TabsTrigger
            value="corivo_proxy"
            disabled={isSaving || !cloudAuthAvailable}
          >
            {t.settings.execAgent.tabCorivo}
          </TabsTrigger>
          <TabsTrigger value="byok" disabled={isSaving}>
            {t.settings.execAgent.tabByok}
          </TabsTrigger>
          <TabsTrigger value="chatgpt" disabled={isSaving}>
            {t.settings.execAgent.tabChatgpt}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="corivo_proxy" className="space-y-6">
          <p className="text-xs text-muted-foreground">
            {t.settings.execAgent.modes.corivo.description}
          </p>

          <FieldGroup title={t.settings.execAgent.modelGroup}>
            <div className="space-y-3 text-sm">
              <div className="flex items-center justify-between">
                <Label className="text-sm">{t.settings.execAgent.mainModelLabel}</Label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => refreshModels.mutate()}
                  disabled={refreshModels.isPending}
                  title={t.settings.execAgent.refreshTitle}
                >
                  <RefreshCw
                    className={`mr-1 h-3 w-3 ${
                      refreshModels.isPending ? "animate-spin" : ""
                    }`}
                  />
                  {t.settings.execAgent.refresh}
                </Button>
              </div>

              {managed.length === 0 ? (
                <p className="rounded-md border border-border/40 bg-muted/30 p-3 text-xs text-muted-foreground">
                  {modelsLoading
                    ? t.settings.execAgent.modelsLoading
                    : t.settings.execAgent.modelsEmpty}
                </p>
              ) : (
                <div className="space-y-2">
                  {managed.map((model) => (
                    <ModelOption
                      key={model.alias}
                      model={model}
                      isDefault={model.alias === directory?.default_alias}
                      checked={selectedModelId === model.alias}
                      disabled={isSaving}
                      onChange={() => setSelectedModelId(model.alias)}
                    />
                  ))}
                </div>
              )}
            </div>
          </FieldGroup>

          <FieldGroup title={t.settings.execAgent.corivoLoginGroup}>
            <div className="space-y-2 text-sm">
              <div className="flex items-center justify-between rounded-md border border-border/40 p-3">
                <div className="space-y-1">
                  <div className="font-medium">
                    {auth?.loggedIn
                      ? t.settings.execAgent.loggedIn
                      : t.settings.execAgent.loggedOut}
                  </div>
                  {auth?.label ? (
                    <div className="text-xs text-muted-foreground">
                      {t.settings.execAgent.accountLabel(auth.label)}
                    </div>
                  ) : !auth?.loggedIn ? (
                    <div className="text-xs text-muted-foreground">
                      {t.settings.execAgent.pleaseLogin}
                    </div>
                  ) : null}
                </div>
                {auth?.loggedIn ? (
                  <Button
                    variant="outline"
                    onClick={() => logout.mutate()}
                    disabled={logout.isPending}
                  >
                    {t.settings.execAgent.logoutCta}
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    onClick={() => void navigate({ to: "/login" })}
                  >
                    {t.settings.execAgent.goLoginCta}
                  </Button>
                )}
              </div>
            </div>
          </FieldGroup>
        </TabsContent>

        <TabsContent value="byok" className="space-y-6">
          <p className="text-xs text-muted-foreground">
            {t.settings.execAgent.modes.byok.description}
          </p>

          <FieldGroup title={t.settings.execAgent.byokGroup}>
            <div className="space-y-3 rounded-md border border-border/40 p-3 text-sm">
              <div className="space-y-2">
                <Label htmlFor="byok-api-shape">
                  {t.settings.execAgent.byokApiShapeLabel}
                </Label>
                <select
                  id="byok-api-shape"
                  value={byokApiShape}
                  disabled={isSaving}
                  onChange={(event) =>
                    setByokField("byok_api_shape", event.target.value)
                  }
                  className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  {API_SHAPES.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="byok-base-url">
                  {t.settings.execAgent.byokBaseUrlLabel}
                </Label>
                <Input
                  id="byok-base-url"
                  type="text"
                  value={byokBaseUrl}
                  placeholder={
                    API_SHAPES.find((s) => s.value === byokApiShape)
                      ?.placeholder
                  }
                  onChange={(event) =>
                    setByokField("byok_base_url", event.target.value)
                  }
                />
                <p className="text-xs text-muted-foreground">
                  {t.settings.execAgent.byokBaseUrlHint}
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="byok-key">{t.settings.execAgent.byokKeyLabel}</Label>
                <Input
                  id="byok-key"
                  type="password"
                  value={byokKey}
                  placeholder={
                    byokApiShape === "anthropic"
                      ? "sk-ant-api03-..."
                      : "sk-..."
                  }
                  onChange={(event) =>
                    setByokField("byok_key", event.target.value)
                  }
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="byok-model">
                  {t.settings.execAgent.byokModelLabel}
                </Label>
                <Input
                  id="byok-model"
                  type="text"
                  value={byokModel}
                  placeholder={
                    byokApiShape === "anthropic"
                      ? "claude-sonnet-4-20250514"
                      : "gpt-4o"
                  }
                  onChange={(event) =>
                    setByokField("byok_model", event.target.value)
                  }
                />
                <p className="text-xs text-muted-foreground">
                  {t.settings.execAgent.byokModelHint}
                </p>
              </div>
            </div>
          </FieldGroup>
        </TabsContent>

        <TabsContent value="chatgpt" className="space-y-6">
          <p className="text-xs text-muted-foreground">
            {t.settings.execAgent.modes.chatgpt.description}
          </p>

          <FieldGroup title={t.settings.execAgent.chatgptGroup}>
            <div className="space-y-2 text-sm">
              <div className="flex items-center justify-between rounded-md border border-border/40 p-3">
                <div className="space-y-1">
                  <div className="font-medium">
                    {chatgptAuth?.signedIn
                      ? t.settings.execAgent.chatgptSignedIn
                      : t.settings.execAgent.chatgptSignedOut}
                  </div>
                  {chatgptAuth?.email ? (
                    <div className="text-xs text-muted-foreground">
                      {t.settings.execAgent.chatgptEmailLabel(chatgptAuth.email)}
                    </div>
                  ) : null}
                  {chatgptAuth?.planType ? (
                    <div className="text-xs text-muted-foreground">
                      {t.settings.execAgent.chatgptPlanLabel(chatgptAuth.planType)}
                    </div>
                  ) : null}
                </div>
                {chatgptAuth?.signedIn ? (
                  <Button
                    variant="outline"
                    onClick={() => chatgptLogout.mutate()}
                    disabled={chatgptLogout.isPending}
                  >
                    {t.settings.execAgent.chatgptLogoutCta}
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    onClick={() => chatgptLogin.mutate()}
                    disabled={chatgptLogin.isPending}
                  >
                    {chatgptLogin.isPending
                      ? t.settings.execAgent.chatgptLoginInProgress
                      : t.settings.execAgent.chatgptLoginCta}
                  </Button>
                )}
              </div>
              <p className="text-xs text-muted-foreground">
                {t.settings.execAgent.chatgptExperimentalHint}
              </p>
            </div>
          </FieldGroup>
        </TabsContent>
      </Tabs>

      <FieldGroup title={t.settings.execAgent.thinkingBudgetGroup}>
        <div className="space-y-2 text-sm">
          <Label htmlFor="thinking-level">{t.settings.execAgent.thinkingLevelLabel}</Label>
          <select
            id="thinking-level"
            value={thinkingLevel}
            disabled={isSaving}
            onChange={(event) =>
              setThinkingLevel(event.target.value as ThinkingLevel)
            }
            className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            {THINKING_LEVELS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {t.settings.execAgent.thinkingLevels[opt.value]}
              </option>
            ))}
          </select>
          <p className="text-xs text-muted-foreground">
            {t.settings.execAgent.thinkingHint}
          </p>
        </div>
      </FieldGroup>
    </div>
  );
}

function ModelOption({
  model,
  isDefault,
  checked,
  disabled,
  onChange,
}: {
  model: ModelMeta;
  isDefault: boolean;
  checked: boolean;
  disabled: boolean;
  onChange: () => void;
}) {
  // DOM id must be a valid HTML identifier; aliases like "corivo:fast"
  // would otherwise put a colon in the attribute. Replace conservatively.
  const id = `model-${model.alias.replace(/[^a-z0-9_-]/gi, "-")}`;
  const caps = model.capabilities;
  return (
    <label
      htmlFor={id}
      className="flex cursor-pointer items-start gap-3 rounded-md border border-border/40 p-3 hover:bg-accent/30"
    >
      <input
        id={id}
        type="radio"
        name="exec-agent-selected-model"
        checked={checked}
        disabled={disabled}
        onChange={onChange}
        className="mt-1"
      />
      <div className="flex-1 space-y-0.5">
        <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
          <span>{model.display_name}</span>
          {/* api_type surfaces the true upstream (e.g. "gemini") even
              when client_protocol is "openai" via sub2api normalization. */}
          <span className="rounded bg-muted px-1.5 text-[10px] text-muted-foreground">
            {model.api_type}
          </span>
          {caps.reasoning ? (
            <span className="rounded bg-muted px-1.5 text-[10px] text-muted-foreground">
              thinking
            </span>
          ) : null}
          {caps.vision ? (
            <span className="rounded bg-muted px-1.5 text-[10px] text-muted-foreground">
              vision
            </span>
          ) : null}
          {isDefault ? (
            <span className="rounded bg-primary/15 px-1.5 text-[10px] text-primary">
              默认
            </span>
          ) : null}
        </div>
        <div className="text-xs text-muted-foreground">
          {model.alias} · {(caps.context_window / 1000).toFixed(0)}k context
        </div>
      </div>
    </label>
  );
}

