import { useState } from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { CheckCircle2, ExternalLink, Loader2, ShieldCheck } from "lucide-react"
import { toast } from "sonner"
import type {
  CategoryToggles,
  ModelDownloadProgress,
  PiiLabel,
  PrivacyModelStatus,
  PrivacySettings,
} from "@corivo/shared-types"

import { Button } from "@repo/ui/components/button"
import { Label } from "@repo/ui/components/label"
import {
  clearPrivacyCache,
  deletePrivacyModel,
  downloadPrivacyModel,
  getPrivacyModelStatus,
  getPrivacySettings,
  setPrivacySettings,
} from "@/lib/tauri"

import { FieldGroup, SectionHeader, SettingsSkeleton } from "./settings-shared"

// React Query keys —— 集中放,免得 typo
const STATUS_KEY = ["privacy-model-status"] as const
const SETTINGS_KEY = ["privacy-settings"] as const

/**
 * PII filter Settings 入口。
 *
 * 状态机:
 *   1. 模型未下载, enabled=false (默认/初始)
 *      → 显示成本卡片(810MB 磁盘 + 1.5GB 内存 + HF 来源链接),
 *        给"启用并下载"按钮
 *   2. 正在下载
 *      → 进度条,显示当前文件 + 百分比
 *      → 下载失败 toast 错误,回到 1
 *   3. 模型已下载, enabled=true
 *      → 主开关 + 8 个类目复选框(secret 强制开)+ 维护按钮
 *   4. 模型已下载, enabled=false (用户主动关掉)
 *      → 主开关 + "删除模型释放磁盘"链接
 *
 * 不显式做"取消下载"——下载是串行的,完成通常很快;后续如有需要
 * 可加 abort signal。
 */
export function PrivacyFilterSection() {
  const queryClient = useQueryClient()

  const status = useQuery<PrivacyModelStatus>({
    queryKey: STATUS_KEY,
    queryFn: getPrivacyModelStatus,
  })
  const settings = useQuery<PrivacySettings>({
    queryKey: SETTINGS_KEY,
    queryFn: getPrivacySettings,
  })

  // 下载进度的本地 state —— 不持久化,完成后清空
  const [progress, setProgress] = useState<ModelDownloadProgress | null>(null)

  const setSettingsMutation = useMutation({
    mutationFn: (next: PrivacySettings) => setPrivacySettings(next),
    onSuccess: (saved) => {
      queryClient.setQueryData(SETTINGS_KEY, saved)
    },
    onError: (error) => toast.error(`保存失败: ${String(error)}`),
  })

  const downloadMutation = useMutation({
    mutationFn: async () => {
      setProgress(null)
      await downloadPrivacyModel((event) => setProgress(event))
    },
    onSuccess: async () => {
      setProgress(null)
      // 下载完成后:刷新状态 + 自动启用
      await queryClient.invalidateQueries({ queryKey: STATUS_KEY })
      if (settings.data) {
        setSettingsMutation.mutate({ ...settings.data, enabled: true })
      }
      toast.success("模型下载完成,PII 过滤已启用")
    },
    onError: (error) => {
      setProgress(null)
      toast.error(`下载失败: ${String(error)}`)
    },
  })

  const deleteMutation = useMutation({
    mutationFn: deletePrivacyModel,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: STATUS_KEY })
      await queryClient.invalidateQueries({ queryKey: SETTINGS_KEY })
      toast.success("模型已删除,PII 过滤已关闭")
    },
    onError: (error) => toast.error(`删除失败: ${String(error)}`),
  })

  const clearCacheMutation = useMutation({
    mutationFn: clearPrivacyCache,
    onSuccess: () => toast.success("识别缓存已清空"),
    onError: (error) => toast.error(`清空失败: ${String(error)}`),
  })

  if (status.isLoading || settings.isLoading) {
    return (
      <div className="max-w-xl space-y-8">
        <SectionHeader title="PII 过滤" description="加载中…" />
        <SettingsSkeleton />
      </div>
    )
  }

  const modelStatus = status.data
  const currentSettings = settings.data
  if (!modelStatus || !currentSettings) {
    return (
      <div className="max-w-xl space-y-8">
        <SectionHeader title="PII 过滤" description="无法读取状态" />
      </div>
    )
  }

  const isDownloading = downloadMutation.isPending
  const isDownloaded = modelStatus.downloaded
  const isEnabled = currentSettings.enabled

  return (
    <div className="max-w-xl space-y-8">
      <SectionHeader
        title="PII 过滤"
        description="把屏幕文本发给 AI 之前,先在本地用 OpenAI privacy-filter 模型识别人名、邮箱、电话等敏感信息并打码。整个过程在本地完成,文本不会因为这次检测而离开你的设备。"
      />

      {/* 主开关 + 状态卡 */}
      <FieldGroup title="启用状态">
        {!isDownloaded ? (
          <NotDownloadedCard
            status={modelStatus}
            isDownloading={isDownloading}
            progress={progress}
            onDownload={() => downloadMutation.mutate()}
          />
        ) : (
          <DownloadedToggleCard
            enabled={isEnabled}
            status={modelStatus}
            onToggle={(next) => {
              if (!currentSettings) return
              setSettingsMutation.mutate({ ...currentSettings, enabled: next })
            }}
            saving={setSettingsMutation.isPending}
          />
        )}
      </FieldGroup>

      {/* 类目控制 —— 仅模型已下载 + 启用时显示 */}
      {isDownloaded && isEnabled ? (
        <FieldGroup title="检测类目">
          <p className="mb-3 text-xs text-muted-foreground">
            命中的类目会在发送给 AI 时被替换成占位符(例如 <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">[人名]</code> / <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">[邮箱]</code>)。
            <strong className="text-foreground">密钥</strong>类目永远启用,而且会在写入本地存储之前就被替换,不可关闭。
          </p>
          <CategoriesEditor
            categories={currentSettings.categories}
            onChange={(next) =>
              setSettingsMutation.mutate({
                ...currentSettings,
                categories: next,
              })
            }
            saving={setSettingsMutation.isPending}
          />
        </FieldGroup>
      ) : null}

      {/* 维护 —— 仅模型已下载时显示 */}
      {isDownloaded ? (
        <FieldGroup title="维护">
          <div className="space-y-2">
            <MaintenanceRow
              title="清除识别缓存"
              description="清空本地保存的「文本 hash → 识别结果」LRU。模型升级或怀疑误判时使用。"
              actionLabel="清除"
              onAction={() => clearCacheMutation.mutate()}
              pending={clearCacheMutation.isPending}
            />
            <MaintenanceRow
              title={`删除模型 (释放 ${formatBytes(modelStatus.total_size_bytes)})`}
              description="删除已下载的模型文件,同时关闭 PII 过滤。需要时可重新下载。"
              actionLabel="删除"
              variant="destructive"
              onAction={() => deleteMutation.mutate()}
              pending={deleteMutation.isPending}
            />
          </div>
        </FieldGroup>
      ) : null}
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/*  Sub components                                                            */
/* -------------------------------------------------------------------------- */

function NotDownloadedCard({
  status,
  isDownloading,
  progress,
  onDownload,
}: {
  status: PrivacyModelStatus
  isDownloading: boolean
  progress: ModelDownloadProgress | null
  onDownload: () => void
}) {
  const sizeLabel = formatBytes(status.total_size_bytes)
  const ramLabel = formatBytes(status.ram_estimate_bytes)

  return (
    <div className="space-y-4 rounded-md border border-border bg-card px-4 py-4">
      <div className="flex items-start gap-3">
        <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />
        <div className="space-y-2 text-sm leading-relaxed">
          <p className="font-medium text-foreground">
            启用前请知悉以下成本:
          </p>
          <ul className="space-y-1.5 text-muted-foreground">
            <li>
              • 需要下载模型文件:约 <strong className="text-foreground">{sizeLabel}</strong>
              (从 Hugging Face 拉取)
            </li>
            <li>
              • 启用后模型常驻内存,大约占用{" "}
              <strong className="text-foreground">{ramLabel} 内存</strong>
            </li>
            <li>
              • 模型来源:{" "}
              <a
                href={status.source_url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-0.5 text-foreground underline-offset-2 hover:underline"
              >
                openai/privacy-filter
                <ExternalLink className="h-3 w-3" />
              </a>
              {" · "}
              variant: <code className="font-mono text-[11px]">{status.variant}</code>
            </li>
          </ul>
        </div>
      </div>

      {isDownloading && progress ? (
        <DownloadProgressBar progress={progress} />
      ) : (
        <div className="flex justify-end">
          <Button onClick={onDownload} disabled={isDownloading}>
            {isDownloading ? (
              <>
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                下载中…
              </>
            ) : (
              <>启用并下载 ({sizeLabel})</>
            )}
          </Button>
        </div>
      )}
    </div>
  )
}

function DownloadProgressBar({ progress }: { progress: ModelDownloadProgress }) {
  // bigint → number 在 GB 量级安全。
  const total = Number(progress.total_bytes)
  const done = Number(progress.downloaded_bytes)
  const ratio = total > 0 ? Math.min(1, done / total) : 0
  const pct = Math.round(ratio * 100)
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span className="font-mono">
          {progress.file_index}/{progress.file_count} · {progress.file_name}
        </span>
        <span className="font-mono">{pct}%</span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className="h-full bg-[var(--corivo-amber)] transition-[width] duration-150"
          style={{ width: `${pct}%` }}
        />
      </div>
      <p className="text-[10px] text-muted-foreground">
        {formatBytes(progress.downloaded_bytes)} / {formatBytes(progress.total_bytes)}
      </p>
    </div>
  )
}

function DownloadedToggleCard({
  enabled,
  status,
  onToggle,
  saving,
}: {
  enabled: boolean
  status: PrivacyModelStatus
  onToggle: (next: boolean) => void
  saving: boolean
}) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-md border border-border bg-card px-4 py-3.5">
      <div className="flex items-start gap-3">
        <CheckCircle2
          className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${
            enabled ? "bg-[var(--corivo-amber)]" : "bg-muted-foreground/40"
          }`}
        />
        <div>
          <Label className="text-sm">
            {enabled ? "PII 过滤已启用" : "PII 过滤已关闭"}
          </Label>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {enabled
              ? `发送到 AI 之前会自动识别 + 替换敏感信息。模型占用约 ${formatBytes(status.ram_estimate_bytes)} 内存。`
              : "敏感信息会原样发送给 AI。"}
          </p>
        </div>
      </div>
      <Button
        size="sm"
        variant={enabled ? "outline" : "default"}
        disabled={saving}
        onClick={() => onToggle(!enabled)}
      >
        {saving ? "保存中…" : enabled ? "关闭" : "启用"}
      </Button>
    </div>
  )
}

const CATEGORY_OPTIONS: Array<{ key: keyof CategoryToggles; label: PiiLabel; display: string; note?: string }> = [
  { key: "private_person", label: "private_person", display: "人名" },
  { key: "private_email", label: "private_email", display: "邮箱" },
  { key: "private_phone", label: "private_phone", display: "电话" },
  { key: "private_address", label: "private_address", display: "地址" },
  { key: "account_number", label: "account_number", display: "账号" },
  { key: "private_url", label: "private_url", display: "URL", note: "默认关闭,误报较多" },
  { key: "private_date", label: "private_date", display: "日期", note: "默认关闭,误报较多" },
  { key: "secret", label: "secret", display: "密钥", note: "强制开启,无法关闭" },
]

function CategoriesEditor({
  categories,
  onChange,
  saving,
}: {
  categories: CategoryToggles
  onChange: (next: CategoryToggles) => void
  saving: boolean
}) {
  return (
    <div className="grid grid-cols-2 gap-2">
      {CATEGORY_OPTIONS.map((opt) => {
        const checked = categories[opt.key]
        const isSecret = opt.key === "secret"
        return (
          <label
            key={opt.key}
            className={`flex cursor-pointer items-start gap-2 rounded-md border border-border bg-card px-3 py-2.5 transition-colors hover:bg-muted/40 ${
              isSecret ? "cursor-not-allowed opacity-80" : ""
            }`}
          >
            <input
              type="checkbox"
              checked={checked}
              disabled={isSecret || saving}
              onChange={(e) =>
                onChange({
                  ...categories,
                  [opt.key]: e.target.checked,
                })
              }
              className="mt-0.5 h-3.5 w-3.5 rounded border-border accent-[var(--corivo-amber)]"
            />
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium">{opt.display}</div>
              {opt.note ? (
                <div className="text-[11px] text-muted-foreground">{opt.note}</div>
              ) : null}
            </div>
          </label>
        )
      })}
    </div>
  )
}

function MaintenanceRow({
  title,
  description,
  actionLabel,
  variant = "outline",
  onAction,
  pending,
}: {
  title: string
  description: string
  actionLabel: string
  variant?: "outline" | "destructive"
  onAction: () => void
  pending: boolean
}) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-md border border-border bg-card px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{title}</div>
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      <Button
        size="sm"
        variant={variant === "destructive" ? "destructive" : "outline"}
        disabled={pending}
        onClick={onAction}
      >
        {pending ? "…" : actionLabel}
      </Button>
    </div>
  )
}

function formatBytes(bytes: number | bigint): string {
  // ts-rs maps Rust u64 → bigint. 模型文件大小在 GB 量级,Number 精度足够。
  const n = typeof bytes === "bigint" ? Number(bytes) : bytes
  if (n <= 0) return "0 B"
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(0)} MB`
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`
}
