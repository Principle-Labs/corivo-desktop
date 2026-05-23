# spec-09-settings-complete.md

## 一、目标

补齐配置页剩余四个子页的完整实现。完成本 spec 后：

- **常规**：开机自启、窗口关闭行为、通知开关、主题、语言、捕获默认值
- **存储**：截图目录可视化、空间占用、按天清理、设置上限
- **隐私**：数据位置说明、单项清理（删所有截图 / 删所有记忆）、权限状态
- **关于**：版本号、更新检查、开源链接、反馈

spec-02 已经完成的 **API Keys** 和 spec-mvp 已经完成的 **测试** 两个子页保持不动，本 spec 在它们旁边补齐其他子页。

## 二、不做什么

- ❌ 不做多语言（i18n）实际翻译（只做开关和骨架，P1 加英文）
- ❌ 不做主题切换的实际逻辑（暖化主题定了就用柔和，深色模式骨架预留）
- ❌ 不做自动更新（Sparkle / tauri-updater 需要代码签名 + appcast，独立 spec）
- ❌ 不做数据导出（P1）
- ❌ 不做账号 / 登录（永远不做）
- ❌ 不做 Prompt 模板的富编辑器（普通 Textarea 够用）

## 三、成功标准

1. 进入「常规」子页能切换所有开关，设置立即生效并持久化（重启 app 保留）
2. 开机自启开关打开后，退出 app 重启系统确实自动启动
3. 进入「存储」子页能看到 captures 目录当前大小、session 数、截图数，且「打开目录」能弹出 Finder
4. 点「清理 7 天前」能真实删除旧 session，列表立刻刷新
5. 设置最大存储上限（GB）后超过时能自动清理（后台任务每天执行一次）
6. 进入「隐私」子页能看到三个数据类别：config.json、captures、Supermemory 记忆，每个都有"查看"和"清理"
7. 清理动作有二次确认 Dialog，避免误删
8. 进入「关于」子页能看到版本号、macOS 系统版本、开源仓库链接、反馈邮箱
9. 子页之间切换流畅，不重新渲染整个页面

## 四、后端改动

### 4.1 Config 结构扩展

回顾 spec-02 的 `Config` 结构，把 `AppConfig` 和 `CaptureConfig` 稍微扩展：

`src-tauri/src/domain/config.rs` 更新：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub interval_secs: u64,
    pub batch_size: usize,              // 从 segment_duration_mins 改名
    pub max_storage_gb: u64,
    pub jpeg_quality: u8,               // 新增，默认 75
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub auto_start: bool,
    pub minimize_to_tray: bool,
    pub notifications_enabled: bool,
    pub theme: ThemeOption,
    pub language: Language,              // 新增
    pub start_capture_on_launch: bool,   // 新增：启动 app 时自动开始捕获
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Zh,
    En,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interval_secs: 15,
            batch_size: 5,
            max_storage_gb: 5,
            jpeg_quality: 75,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            minimize_to_tray: true,
            notifications_enabled: true,
            theme: ThemeOption::System,
            language: Language::Zh,
            start_capture_on_launch: false,
        }
    }
}
```

**config.json 的迁移**：老版本的 config 缺少新字段，serde 的 `#[serde(default)]` 能处理。在每个新字段加：

```rust
#[serde(default)]
pub jpeg_quality: u8,
```

或给 struct 加 `#[serde(default)]` 让缺失字段全走 `Default::default()`。

### 4.2 开机自启

使用 `tauri-plugin-autostart`：

```toml
# Cargo.toml
tauri-plugin-autostart = "2"
```

```bash
# 前端
pnpm add @tauri-apps/plugin-autostart
```

`src-tauri/src/lib.rs` 注册：

```rust
.plugin(tauri_plugin_autostart::init(
    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
    Some(vec![]),  // 启动参数
))
```

capabilities/default.json 增加：

```json
"autostart:default"
```

### 4.3 新增 Tauri commands

`src-tauri/src/commands/settings.rs`（新建）：

```rust
use crate::services::capture_store::{CaptureStore, StorageStats};
use crate::services::config_service::ConfigService;
use crate::commands::capture::CaptureAppState;
use crate::commands::config::AppState;
use crate::providers::memory::ListQuery;
use crate::services::memory_service::MemoryService;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
pub async fn set_autostart_enabled(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|e| format!("启用开机自启失败: {}", e))
    } else {
        autostart.disable().map_err(|e| format!("禁用开机自启失败: {}", e))
    }
}

#[tauri::command]
pub async fn get_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| format!("获取开机自启状态失败: {}", e))
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let os_version = sys_info::os_release()
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(SystemInfo {
        os,
        arch,
        os_version,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        tauri_version: "2".to_string(),
    })
}

#[derive(Debug, serde::Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub os_version: String,
    pub app_version: String,
    pub tauri_version: String,
}

/// 清理所有截图（保留 session 索引但删所有图）
#[tauri::command]
pub async fn clear_all_screenshots(
    state: State<'_, CaptureAppState>,
) -> Result<i64, String> {
    let sessions = state
        .capture_store
        .list_sessions()
        .await
        .map_err(|e| String::from(e))?;
    
    let mut total = 0;
    for s in sessions {
        if let Err(e) = state.capture_store.delete_session(&s.id).await {
            tracing::error!("删除 session {} 失败: {:?}", s.id, e);
        } else {
            total += s.screenshot_count;
        }
    }
    Ok(total)
}

/// 清理所有 Supermemory 记忆（通过 provider 批量删除）
/// 注意：这是一个危险操作，需要前端二次确认
#[tauri::command]
pub async fn clear_all_memories(
    state: State<'_, crate::commands::memory::MemoryAppState>,
) -> Result<i64, String> {
    let mut deleted = 0;
    let mut offset = 0;
    let batch_size = 50;
    
    loop {
        let page = state
            .memory_service
            .list(ListQuery {
                limit: batch_size,
                offset: 0,  // 始终 0，因为每次删完旧数据后下一批就变成新的第一页
                date_from: None,
                date_to: None,
                source_kind: None,
            })
            .await
            .map_err(|e| String::from(e))?;
        
        if page.items.is_empty() {
            break;
        }
        
        for m in &page.items {
            if let Err(e) = state.memory_service.delete(&m.id).await {
                tracing::error!("删除记忆 {} 失败: {:?}", m.id, e);
                offset += 1;  // 避免死循环：删不掉的往后偏移
            } else {
                deleted += 1;
            }
        }
        
        if offset > 10 {
            return Err("连续删除失败超过 10 次，已中止".to_string());
        }
    }
    
    Ok(deleted)
}

#[tauri::command]
pub async fn check_screen_recording_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        // 尝试截图，成功即有权限
        use xcap::Monitor;
        match Monitor::all() {
            Ok(monitors) => {
                if let Some(m) = monitors.into_iter().next() {
                    Ok(m.capture_image().is_ok())
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(true)
    }
}

#[tauri::command]
pub async fn open_system_settings_privacy() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn()
            .map_err(|e| format!("打开系统设置失败: {}", e))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "ms-settings:privacy-camera"])
            .spawn()
            .map_err(|e| format!("打开系统设置失败: {}", e))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        Err("Linux 下请手动检查截图相关权限".to_string())
    }
}
```

新增依赖：

```toml
sys-info = "0.9"
```

注册：

```rust
commands::settings::set_autostart_enabled,
commands::settings::get_autostart_enabled,
commands::settings::get_system_info,
commands::settings::clear_all_screenshots,
commands::settings::clear_all_memories,
commands::settings::check_screen_recording_permission,
commands::settings::open_system_settings_privacy,
```

### 4.4 后台存储清理任务

当 `max_storage_gb` 超出时自动清理。

`src-tauri/src/services/storage_cleanup.rs`（新建）：

```rust
use crate::services::capture_store::CaptureStore;
use crate::services::config_service::ConfigService;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub fn start_cleanup_task(
    config_service: Arc<ConfigService>,
    capture_store: Arc<CaptureStore>,
) {
    tokio::spawn(async move {
        // 启动延迟 1 分钟再跑第一次，避免启动时 IO 密集
        sleep(Duration::from_secs(60)).await;
        
        loop {
            if let Err(e) = run_cleanup_cycle(&config_service, &capture_store).await {
                tracing::error!("storage cleanup failed: {:?}", e);
            }
            // 每 6 小时检查一次
            sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn run_cleanup_cycle(
    config_service: &ConfigService,
    capture_store: &CaptureStore,
) -> crate::error::Result<()> {
    let cfg = config_service.get();
    let max_bytes = (cfg.capture.max_storage_gb as u64) * 1024 * 1024 * 1024;
    
    let stats = capture_store.get_storage_stats().await?;
    if (stats.total_size_bytes as u64) <= max_bytes {
        return Ok(());
    }
    
    tracing::info!(
        "storage {} exceeds limit {}, cleaning oldest sessions",
        stats.total_size_bytes,
        max_bytes
    );
    
    // 按旧到新删除 session，直到总大小降到 80% 水位线
    let target_bytes = (max_bytes as f64 * 0.8) as i64;
    
    let mut sessions = capture_store.list_sessions().await?;
    sessions.sort_by_key(|s| s.started_at);  // 旧的在前
    
    let mut current_size = stats.total_size_bytes;
    for s in sessions {
        if current_size <= target_bytes {
            break;
        }
        let before = capture_store
            .get_storage_stats()
            .await
            .map(|x| x.total_size_bytes)
            .unwrap_or(current_size);
        
        if let Err(e) = capture_store.delete_session(&s.id).await {
            tracing::error!("cleanup 删除 session {} 失败: {:?}", s.id, e);
            continue;
        }
        
        let after = capture_store
            .get_storage_stats()
            .await
            .map(|x| x.total_size_bytes)
            .unwrap_or(current_size);
        
        current_size = after;
        tracing::info!("cleaned session {} ({} bytes freed)", s.id, before - after);
    }
    
    Ok(())
}
```

在 `lib.rs` setup 里启动：

```rust
use services::storage_cleanup;
storage_cleanup::start_cleanup_task(
    config_service.clone(),
    capture_store.clone(),
);
```

### 4.5 set_config 的副作用

用户在设置页改 `capture.interval_secs` / `batch_size` 应该立刻生效到 CaptureLoop。修改 `commands/config.rs`：

```rust
#[tauri::command]
pub async fn set_config(
    state: State<'_, AppState>,
    capture_state: State<'_, CaptureAppState>,
    config: Config,
) -> Result<(), String> {
    state.config_service.update(config.clone()).map_err(String::from)?;
    
    // 同步捕获配置到运行中的 CaptureLoop
    let runtime_cfg = crate::services::capture_loop::CaptureRuntimeConfig {
        interval_secs: config.capture.interval_secs,
        batch_size: config.capture.batch_size,
        jpeg_quality: config.capture.jpeg_quality,
    };
    capture_state.capture_loop.update_config(runtime_cfg).await;
    
    Ok(())
}
```

## 五、前端实现

### 5.1 类型与 API 封装

`src/lib/types.ts` 更新：

```ts
export type ThemeOption = 'light' | 'dark' | 'system'
export type Language = 'zh' | 'en'

export interface CaptureConfig {
  interval_secs: number
  batch_size: number
  max_storage_gb: number
  jpeg_quality: number
}

export interface AppConfig {
  auto_start: boolean
  minimize_to_tray: boolean
  notifications_enabled: boolean
  theme: ThemeOption
  language: Language
  start_capture_on_launch: boolean
}

export interface Config {
  capture: CaptureConfig
  summary: SummaryConfig
  app: AppConfig
  memory: MemoryConfig
}

export interface SystemInfo {
  os: string
  arch: string
  os_version: string
  app_version: string
  tauri_version: string
}
```

`src/lib/tauri.ts` 追加：

```ts
import type { SystemInfo } from './types'

export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  return invoke<void>('set_autostart_enabled', { enabled })
}

export async function getAutostartEnabled(): Promise<boolean> {
  return invoke<boolean>('get_autostart_enabled')
}

export async function getSystemInfo(): Promise<SystemInfo> {
  return invoke<SystemInfo>('get_system_info')
}

export async function clearAllScreenshots(): Promise<number> {
  return invoke<number>('clear_all_screenshots')
}

export async function clearAllMemories(): Promise<number> {
  return invoke<number>('clear_all_memories')
}

export async function checkScreenRecordingPermission(): Promise<boolean> {
  return invoke<boolean>('check_screen_recording_permission')
}

export async function openSystemSettingsPrivacy(): Promise<void> {
  return invoke<void>('open_system_settings_privacy')
}
```

### 5.2 共享 hook：useConfig

`src/hooks/use-config.ts`（新建）：

```ts
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { getConfig, setConfig } from '@/lib/tauri'
import type { Config } from '@/lib/types'

export function useConfig() {
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['config'],
    queryFn: getConfig,
    staleTime: Infinity,
  })

  const mutation = useMutation({
    mutationFn: (newConfig: Config) => setConfig(newConfig),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['config'] })
    },
    onError: (e: string) => {
      toast.error(`保存失败：${e}`)
    },
  })

  const update = (patch: Partial<Config> | ((prev: Config) => Config)) => {
    if (!query.data) return
    const next =
      typeof patch === 'function'
        ? patch(query.data)
        : { ...query.data, ...patch }
    mutation.mutate(next)
  }

  return {
    config: query.data,
    isLoading: query.isLoading,
    update,
    isSaving: mutation.isPending,
  }
}
```

### 5.3 设置页主结构更新

`src/pages/settings/settings-page.tsx` 更新 sections：

```tsx
import { GeneralSection } from './sections/general-section'
import { StorageSection } from './sections/storage-section'
import { PrivacySection } from './sections/privacy-section'
import { AboutSection } from './sections/about-section'
// ApiKeysSection 和 TestSection 来自 spec-02 / spec-mvp

type Section = 'general' | 'api-keys' | 'storage' | 'privacy' | 'test' | 'about'

const sections: { id: Section; label: string }[] = [
  { id: 'general', label: '常规' },
  { id: 'api-keys', label: 'API Keys' },
  { id: 'storage', label: '存储' },
  { id: 'privacy', label: '隐私' },
  { id: 'test', label: '测试' },
  { id: 'about', label: '关于' },
]

// render 部分对应每个 section:
{active === 'general' && <GeneralSection />}
{active === 'api-keys' && <ApiKeysSection />}
{active === 'storage' && <StorageSection />}
{active === 'privacy' && <PrivacySection />}
{active === 'test' && <TestSection />}
{active === 'about' && <AboutSection />}
```

### 5.4 需要的 shadcn 组件

```bash
pnpm dlx shadcn@latest add switch select slider alert-dialog badge
```

### 5.5 GeneralSection

`src/pages/settings/sections/general-section.tsx`（新建）：

```tsx
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Slider } from '@/components/ui/slider'
import { useConfig } from '@/hooks/use-config'
import {
  getAutostartEnabled,
  setAutostartEnabled,
} from '@/lib/tauri'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'

export function GeneralSection() {
  const { config, isLoading, update } = useConfig()
  const queryClient = useQueryClient()

  const { data: autostart } = useQuery({
    queryKey: ['autostart'],
    queryFn: getAutostartEnabled,
  })

  const autostartMut = useMutation({
    mutationFn: setAutostartEnabled,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['autostart'] })
      toast.success('已更新开机自启设置')
    },
    onError: (e: string) => toast.error(`设置失败：${e}`),
  })

  if (isLoading || !config) {
    return <SettingsSkeleton />
  }

  return (
    <div className="space-y-8 max-w-xl">
      <SectionHeader
        title="常规"
        description="应用行为、启动和界面偏好"
      />

      <FieldGroup title="启动行为">
        <ToggleRow
          label="开机自启动"
          description="电脑启动时自动运行 Corivo（后台）"
          value={autostart ?? false}
          onChange={(v) => autostartMut.mutate(v)}
        />
        <ToggleRow
          label="启动时自动开始捕获"
          description="Corivo 启动后立即开始屏幕捕获，无需手动点击"
          value={config.app.start_capture_on_launch}
          onChange={(v) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, start_capture_on_launch: v },
            }))
          }
        />
      </FieldGroup>

      <FieldGroup title="窗口与通知">
        <ToggleRow
          label="关闭时最小化到托盘"
          description="点击窗口的关闭按钮时隐藏到系统托盘而不是退出"
          value={config.app.minimize_to_tray}
          onChange={(v) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, minimize_to_tray: v },
            }))
          }
        />
        <ToggleRow
          label="启用系统通知"
          description="允许 Corivo 发送推送通知(关闭后所有推送都不会显示)"
          value={config.app.notifications_enabled}
          onChange={(v) =>
            update((prev) => ({
              ...prev,
              app: { ...prev.app, notifications_enabled: v },
            }))
          }
        />
      </FieldGroup>

      <FieldGroup title="外观与语言">
        <div className="space-y-2">
          <Label className="text-sm">主题</Label>
          <Select
            value={config.app.theme}
            onValueChange={(v) =>
              update((prev) => ({
                ...prev,
                app: { ...prev.app, theme: v as any },
              }))
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="light">浅色</SelectItem>
              <SelectItem value="dark">深色</SelectItem>
              <SelectItem value="system">跟随系统</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            深色模式 P1 支持，当前切换无实际效果
          </p>
        </div>

        <div className="space-y-2">
          <Label className="text-sm">语言</Label>
          <Select
            value={config.app.language}
            onValueChange={(v) =>
              update((prev) => ({
                ...prev,
                app: { ...prev.app, language: v as any },
              }))
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="zh">简体中文</SelectItem>
              <SelectItem value="en">English (P1)</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </FieldGroup>

      <FieldGroup title="捕获默认值">
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-sm">截图间隔</Label>
            <span className="text-xs tabular-nums text-muted-foreground">
              {config.capture.interval_secs} 秒
            </span>
          </div>
          <Slider
            value={[config.capture.interval_secs]}
            min={5}
            max={60}
            step={5}
            onValueChange={([v]) =>
              update((prev) => ({
                ...prev,
                capture: { ...prev.capture, interval_secs: v },
              }))
            }
          />
          <p className="text-xs text-muted-foreground">
            多久截一次屏幕。数值越小越实时，成本越高。
          </p>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-sm">每批截图数</Label>
            <span className="text-xs tabular-nums text-muted-foreground">
              {config.capture.batch_size} 张
            </span>
          </div>
          <Slider
            value={[config.capture.batch_size]}
            min={2}
            max={20}
            step={1}
            onValueChange={([v]) =>
              update((prev) => ({
                ...prev,
                capture: { ...prev.capture, batch_size: v },
              }))
            }
          />
          <p className="text-xs text-muted-foreground">
            攒多少张截图触发一次总结和推送判断。越少越频繁，通知越多。
          </p>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-sm">JPEG 质量</Label>
            <span className="text-xs tabular-nums text-muted-foreground">
              {config.capture.jpeg_quality}
            </span>
          </div>
          <Slider
            value={[config.capture.jpeg_quality]}
            min={40}
            max={95}
            step={5}
            onValueChange={([v]) =>
              update((prev) => ({
                ...prev,
                capture: { ...prev.capture, jpeg_quality: v },
              }))
            }
          />
          <p className="text-xs text-muted-foreground">
            截图压缩质量。75 左右是平衡点，越高占用越大。
          </p>
        </div>
      </FieldGroup>
    </div>
  )
}

function SectionHeader({
  title,
  description,
}: {
  title: string
  description: string
}) {
  return (
    <div>
      <h2 className="text-base font-medium mb-1">{title}</h2>
      <p className="text-xs text-muted-foreground">{description}</p>
    </div>
  )
}

function FieldGroup({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <section className="space-y-4">
      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
        {title}
      </div>
      <div className="space-y-4">{children}</div>
    </section>
  )
}

function ToggleRow({
  label,
  description,
  value,
  onChange,
}: {
  label: string
  description: string
  value: boolean
  onChange: (v: boolean) => void
}) {
  return (
    <div className="flex items-start gap-4">
      <div className="flex-1 min-w-0">
        <Label className="text-sm cursor-pointer">{label}</Label>
        <p className="text-xs text-muted-foreground mt-0.5">{description}</p>
      </div>
      <Switch checked={value} onCheckedChange={onChange} className="mt-0.5" />
    </div>
  )
}

function SettingsSkeleton() {
  return (
    <div className="space-y-4 max-w-xl">
      {[1, 2, 3].map((i) => (
        <div
          key={i}
          className="h-16 bg-muted/40 rounded-md animate-pulse"
        />
      ))}
    </div>
  )
}
```

### 5.6 StorageSection

`src/pages/settings/sections/storage-section.tsx`（新建）：

```tsx
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { useState } from 'react'
import { FolderOpen, Trash2, Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Slider } from '@/components/ui/slider'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { useConfig } from '@/hooks/use-config'
import {
  getStorageStats,
  openDataDirectory,
  cleanupOldSessions,
} from '@/lib/tauri'

export function StorageSection() {
  const { config, update } = useConfig()
  const queryClient = useQueryClient()
  const [cleanupDays, setCleanupDays] = useState(7)

  const { data: stats } = useQuery({
    queryKey: ['storage-stats'],
    queryFn: getStorageStats,
    refetchInterval: 5_000,
  })

  const cleanupMut = useMutation({
    mutationFn: (days: number) => cleanupOldSessions(days),
    onSuccess: (count) => {
      toast.success(`已清理 ${count} 个会话`)
      queryClient.invalidateQueries({ queryKey: ['storage-stats'] })
      queryClient.invalidateQueries({ queryKey: ['sessions'] })
    },
    onError: (e: string) => toast.error(`清理失败：${e}`),
  })

  if (!config || !stats) {
    return <div className="text-sm text-muted-foreground">加载中…</div>
  }

  const sizeMB = (stats.total_size_bytes / 1024 / 1024).toFixed(1)
  const sizeGB = (stats.total_size_bytes / 1024 / 1024 / 1024).toFixed(2)
  const usagePercent =
    (stats.total_size_bytes /
      (config.capture.max_storage_gb * 1024 * 1024 * 1024)) *
    100

  return (
    <div className="space-y-8 max-w-xl">
      <div>
        <h2 className="text-base font-medium mb-1">存储</h2>
        <p className="text-xs text-muted-foreground">
          管理本地截图数据和空间占用
        </p>
      </div>

      <section className="space-y-3">
        <div className="rounded-xl border border-border bg-card p-4 space-y-3">
          <div className="flex items-baseline justify-between">
            <span className="text-xs text-muted-foreground">已占用空间</span>
            <span className="text-xs tabular-nums text-muted-foreground">
              {stats.session_count} 会话 · {stats.screenshot_count} 张截图
            </span>
          </div>
          <div>
            <div className="flex items-baseline gap-1.5">
              <span className="text-2xl font-semibold tabular-nums">
                {Number(sizeMB) >= 1024 ? sizeGB : sizeMB}
              </span>
              <span className="text-sm text-muted-foreground">
                {Number(sizeMB) >= 1024 ? 'GB' : 'MB'}
              </span>
            </div>
            <div className="mt-3 h-1.5 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full bg-primary/60 rounded-full transition-all"
                style={{ width: `${Math.min(usagePercent, 100)}%` }}
              />
            </div>
            <div className="mt-1.5 text-xs text-muted-foreground">
              上限 {config.capture.max_storage_gb} GB ·{' '}
              {usagePercent.toFixed(1)}% 已使用
            </div>
          </div>
        </div>

        <Button
          variant="outline"
          size="sm"
          onClick={() => openDataDirectory()}
          className="gap-2"
        >
          <FolderOpen className="w-3 h-3" />
          打开数据目录
        </Button>
        <p className="text-xs text-muted-foreground font-mono break-all">
          {stats.captures_dir}
        </p>
      </section>

      <section className="space-y-4">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          存储上限
        </div>
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-sm">最大占用</Label>
            <span className="text-xs tabular-nums text-muted-foreground">
              {config.capture.max_storage_gb} GB
            </span>
          </div>
          <Slider
            value={[config.capture.max_storage_gb]}
            min={1}
            max={50}
            step={1}
            onValueChange={([v]) =>
              update((prev) => ({
                ...prev,
                capture: { ...prev.capture, max_storage_gb: v },
              }))
            }
          />
          <p className="text-xs text-muted-foreground">
            超过此值时，Corivo 每天自动清理最早的截图。
          </p>
        </div>
      </section>

      <section className="space-y-4">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          手动清理
        </div>
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <Label className="text-sm">清理 N 天前的截图</Label>
            <span className="text-xs tabular-nums text-muted-foreground">
              {cleanupDays} 天前
            </span>
          </div>
          <Slider
            value={[cleanupDays]}
            min={1}
            max={30}
            step={1}
            onValueChange={([v]) => setCleanupDays(v)}
          />
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                disabled={cleanupMut.isPending}
                className="gap-2"
              >
                {cleanupMut.isPending ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Trash2 className="w-3 h-3" />
                )}
                清理 {cleanupDays} 天前的数据
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>确认清理?</AlertDialogTitle>
                <AlertDialogDescription>
                  这会永久删除 {cleanupDays} 天前的所有截图文件。此操作不可撤销。
                  Supermemory 里的记忆不会受影响。
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>取消</AlertDialogCancel>
                <AlertDialogAction
                  onClick={() => cleanupMut.mutate(cleanupDays)}
                >
                  确认清理
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>
      </section>
    </div>
  )
}
```

### 5.7 PrivacySection

`src/pages/settings/sections/privacy-section.tsx`（新建）：

```tsx
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Camera, FileWarning, Cloud, Trash2, CheckCircle2, XCircle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import {
  checkScreenRecordingPermission,
  openSystemSettingsPrivacy,
  clearAllScreenshots,
  clearAllMemories,
} from '@/lib/tauri'

export function PrivacySection() {
  const queryClient = useQueryClient()

  const { data: hasPermission } = useQuery({
    queryKey: ['screen-permission'],
    queryFn: checkScreenRecordingPermission,
    refetchInterval: 10_000,
  })

  const clearScreenshotsMut = useMutation({
    mutationFn: clearAllScreenshots,
    onSuccess: (count) => {
      toast.success(`已清理 ${count} 张截图`)
      queryClient.invalidateQueries({ queryKey: ['storage-stats'] })
      queryClient.invalidateQueries({ queryKey: ['sessions'] })
    },
    onError: (e: string) => toast.error(`清理失败：${e}`),
  })

  const clearMemoriesMut = useMutation({
    mutationFn: clearAllMemories,
    onSuccess: (count) => {
      toast.success(`已删除 ${count} 条记忆`)
      queryClient.invalidateQueries({ queryKey: ['memories'] })
    },
    onError: (e: string) => toast.error(`删除失败：${e}`),
  })

  return (
    <div className="space-y-8 max-w-xl">
      <div>
        <h2 className="text-base font-medium mb-1">隐私</h2>
        <p className="text-xs text-muted-foreground">
          你的数据存储在哪里，如何控制它们
        </p>
      </div>

      <section className="space-y-3">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          数据流向
        </div>
        <div className="rounded-xl border border-border bg-card p-4 text-xs leading-relaxed text-muted-foreground space-y-2">
          <p>
            Corivo 的所有原始截图保存在你的电脑本地。
            截图通过 Google Gemini API 做文本总结，总结结果存入你的 Supermemory 账号。
          </p>
          <p className="text-foreground">
            Corivo 本身不收集、不上传、不分析你的任何数据。
          </p>
        </div>
      </section>

      <section className="space-y-3">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          系统权限
        </div>
        <div className="rounded-xl border border-border bg-card p-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              {hasPermission ? (
                <CheckCircle2 className="w-4 h-4 text-green-600" />
              ) : (
                <XCircle className="w-4 h-4 text-destructive" />
              )}
              <span className="text-sm font-medium">屏幕录制权限</span>
            </div>
            {!hasPermission && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => openSystemSettingsPrivacy()}
              >
                去授权
              </Button>
            )}
          </div>
          <p className="text-xs text-muted-foreground mt-2">
            {hasPermission
              ? 'Corivo 已获得屏幕录制权限，可以正常工作。'
              : '未获得权限，截图功能无法工作。点右侧按钮去系统设置授权。授权后需要重启 Corivo。'}
          </p>
        </div>
      </section>

      <section className="space-y-3">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          数据清理
        </div>

        <DataClearRow
          icon={<Camera className="w-4 h-4" />}
          title="所有原始截图"
          description="删除本地所有会话的截图文件，但保留 Supermemory 里的记忆总结"
          dangerTitle="删除所有截图?"
          dangerDescription="这会删除 Corivo 本地所有会话的原始截图文件，此操作不可撤销。Supermemory 里已经生成的记忆总结不会受影响。"
          onConfirm={() => clearScreenshotsMut.mutate()}
          isLoading={clearScreenshotsMut.isPending}
        />

        <DataClearRow
          icon={<Cloud className="w-4 h-4" />}
          title="所有 Supermemory 记忆"
          description="删除 Supermemory 账号中 Corivo 创建的所有记忆"
          dangerTitle="删除所有记忆?"
          dangerDescription="这会调用 Supermemory API 批量删除 Corivo 账号下的所有记忆。此操作不可撤销，且需要几分钟才能完成。"
          onConfirm={() => clearMemoriesMut.mutate()}
          isLoading={clearMemoriesMut.isPending}
          destructive
        />
      </section>

      <section className="space-y-3 pt-4 border-t border-border">
        <div className="flex items-start gap-2 text-xs text-muted-foreground">
          <FileWarning className="w-3 h-3 mt-0.5 shrink-0" />
          <p className="leading-relaxed">
            想要完全抹除 Corivo 的痕迹？卸载应用后手动删除
            <span className="font-mono mx-1">~/Library/Application Support/com.corivo.app</span>
            目录即可。
          </p>
        </div>
      </section>
    </div>
  )
}

function DataClearRow({
  icon,
  title,
  description,
  dangerTitle,
  dangerDescription,
  onConfirm,
  isLoading,
  destructive = false,
}: {
  icon: React.ReactNode
  title: string
  description: string
  dangerTitle: string
  dangerDescription: string
  onConfirm: () => void
  isLoading: boolean
  destructive?: boolean
}) {
  return (
    <div className="rounded-xl border border-border bg-card p-4 flex items-start gap-3">
      <div className="mt-0.5 text-muted-foreground">{icon}</div>
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium">{title}</div>
        <p className="text-xs text-muted-foreground mt-0.5">{description}</p>
      </div>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button
            variant={destructive ? 'destructive' : 'outline'}
            size="sm"
            disabled={isLoading}
            className="gap-1.5 shrink-0"
          >
            <Trash2 className="w-3 h-3" />
            清理
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{dangerTitle}</AlertDialogTitle>
            <AlertDialogDescription>{dangerDescription}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              onClick={onConfirm}
              className={
                destructive
                  ? 'bg-destructive text-destructive-foreground hover:bg-destructive/90'
                  : ''
              }
            >
              确认删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
```

### 5.8 AboutSection

`src/pages/settings/sections/about-section.tsx`（新建）：

```tsx
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, Github, Mail } from 'lucide-react'
import { getSystemInfo } from '@/lib/tauri'

export function AboutSection() {
  const { data: info } = useQuery({
    queryKey: ['system-info'],
    queryFn: getSystemInfo,
  })

  return (
    <div className="space-y-8 max-w-xl">
      <div>
        <h2 className="text-base font-medium mb-1">关于 Corivo</h2>
        <p className="text-xs text-muted-foreground">
          本地优先的个人记忆与上下文同步工具
        </p>
      </div>

      <section>
        <div className="rounded-xl border border-border bg-card p-5 text-center">
          <div className="mx-auto w-12 h-12 rounded-xl bg-primary/10 flex items-center justify-center mb-3">
            <span className="text-primary font-semibold text-lg">C</span>
          </div>
          <div className="text-sm font-medium">Corivo</div>
          <div className="text-xs text-muted-foreground mt-0.5">
            版本 {info?.app_version ?? '...'}
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          系统信息
        </div>
        <div className="rounded-xl border border-border bg-card p-4 text-sm space-y-2">
          <InfoRow label="操作系统" value={`${info?.os ?? '...'} ${info?.os_version ?? ''}`} />
          <InfoRow label="架构" value={info?.arch ?? '...'} />
          <InfoRow label="Tauri" value={info?.tauri_version ?? '...'} />
        </div>
      </section>

      <section className="space-y-3">
        <div className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
          资源
        </div>
        <div className="space-y-2">
          <LinkRow
            icon={<Github className="w-3.5 h-3.5" />}
            label="源代码"
            href="https://github.com/your-org/corivo"
          />
          <LinkRow
            icon={<ExternalLink className="w-3.5 h-3.5" />}
            label="文档"
            href="https://corivo.app/docs"
          />
          <LinkRow
            icon={<Mail className="w-3.5 h-3.5" />}
            label="反馈邮箱"
            href="mailto:feedback@corivo.app"
          />
        </div>
      </section>

      <section className="pt-4 border-t border-border text-xs text-muted-foreground leading-relaxed">
        <p>Corivo 开源于 MIT 协议。欢迎提交 issue 和 PR。</p>
      </section>
    </div>
  )
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-xs tabular-nums">{value}</span>
    </div>
  )
}

function LinkRow({
  icon,
  label,
  href,
}: {
  icon: React.ReactNode
  label: string
  href: string
}) {
  return (
    
      href={href}
      target="_blank"
      rel="noreferrer"
      className="flex items-center gap-2 px-3 py-2 rounded-md text-sm hover:bg-accent/40 transition-colors"
    >
      <span className="text-muted-foreground">{icon}</span>
      <span>{label}</span>
      <ExternalLink className="w-3 h-3 text-muted-foreground ml-auto" />
    </a>
  )
}
```

## 六、窗口关闭行为接入

`minimize_to_tray` 开关需要在窗口关闭事件里生效。

`src-tauri/src/lib.rs` setup 里：

```rust
let cfg_for_close = config_service.clone();
app.on_window_event(move |window, event| {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        let cfg = cfg_for_close.get();
        if cfg.app.minimize_to_tray {
            api.prevent_close();
            let _ = window.hide();
        }
    }
});
```

spec-12 做系统托盘时，托盘菜单需要有"退出"按钮彻底关闭 app。本 spec 只做到"最小化到托盘就是隐藏窗口"。

## 七、任务分解

| 会话 | 范围 | 完成判定 |
|---|---|---|
| 1 | Config 结构扩展 + autostart plugin 注册 + set_config 副作用 | cargo build 通过 |
| 2 | commands/settings.rs 全部 command + sys-info 依赖 | invoke 全部命令无报错 |
| 3 | storage_cleanup 后台任务 + lib.rs 启动钩子 | 手动改 max 为 0.001 验证自动清理 |
| 4 | 窗口关闭 minimize_to_tray 逻辑 | 开关生效 |
| 5 | 前端 types/tauri.ts 更新 + useConfig hook | 编译通过 |
| 6 | shadcn 组件安装 + GeneralSection | 常规子页完整可用 |
| 7 | StorageSection | 存储子页能看到数据能清理 |
| 8 | PrivacySection | 隐私子页权限检查 + 清理按钮 |
| 9 | AboutSection | 关于子页 |
| 10 | settings-page.tsx 主结构整合 + 端到端测试 | 验收清单全过 |

## 八、验收清单

- [ ] 进入配置页默认选中「常规」子页
- [ ] 切换任意开关立即保存，重启 app 后保留
- [ ] 开启开机自启，macOS 系统设置 → 登录项能看到 Corivo
- [ ] 滑动截图间隔滑块，CaptureLoop 立即用新间隔（看日志）
- [ ] 「存储」子页显示的大小和「打开目录」看到的实际大小一致（误差 < 5%）
- [ ] 调低 max_storage_gb 后等 6 小时（或手动 kick 一次 cleanup），超出部分被清理
- [ ] 点「清理 7 天前」弹二次确认 Dialog，点确认后数据真的删除
- [ ] 「隐私」子页显示屏幕权限状态正确（对比系统设置的开关）
- [ ] 点「去授权」能打开 macOS 系统设置的屏幕录制权限页面
- [ ] 清理所有截图/所有记忆都有 Dialog 二次确认
- [ ] 「关于」子页显示正确的 app_version 和系统信息
- [ ] 关闭主窗口时（开启 minimize_to_tray）窗口隐藏而不是退出
- [ ] 所有操作的 toast 提示清晰，成功失败都有反馈
- [ ] 视觉上所有 section 使用统一的 FieldGroup / SectionHeader 结构

## 九、坑点预警

1. **Switch 组件的 controlled vs uncontrolled**：shadcn Switch 支持 `checked + onCheckedChange`。直接把 `useConfig` 返回的值传进去，mutation 会异步更新。**用户连续点 Switch 很快**会有视觉闪烁——接受这个，或者加 local state 做乐观更新（工作量不值）。

2. **autostart plugin 在 macOS 的路径**：它会在 `~/Library/LaunchAgents/` 创建一个 plist。如果用户手动删了这个文件，`is_enabled()` 仍然返回 true 但实际不启动。这是 plugin 的已知问题，P0 忽略。

3. **`tauri-plugin-autostart` 的打包差异**：dev 模式下 autostart 会指向 `target/debug/...` 的二进制，build 后会指向 `.app`。开发阶段开 autostart 意义不大，建议 dev 时不开。

4. **`sys-info` crate 的 OS 版本格式**：macOS 上返回 `Darwin 24.x.x`（内核版本），不是 `macOS 15.x`。用户会困惑。**可接受的降级**：在「关于」页显示 macOS 版本时在旁边加一行小字"Darwin 内核版本"。或者用 Swift FFI 读真实版本（P1 再说）。

5. **`clear_all_memories` 可能失败一半**：批量删除中途失败没有事务。如果删到一半网络断了，Supermemory 里会剩一部分。P0 接受——toast 会提示删了多少条，用户可以再点一次继续。

6. **存储自动清理的时机**：我设了启动后 1 分钟 + 每 6 小时一次。如果用户一天只开 2 小时 app，可能永远触发不到 6 小时检查。**可接受**——用户手动清理也行。P1 改成每次 app 启动检查一次。

7. **权限检查的成本**：`checkScreenRecordingPermission` 每次都调一次 `capture_image` 做测试，`refetchInterval: 10_000` 就是每 10 秒截图一次丢弃。CPU 和磁盘 IO 的代价很小（不写盘），但有顾虑的话改成 30 秒或 60 秒。

8. **Alert Dialog 的键盘操作**：Esc 关闭、Enter 不会默认触发 action（安全设计）。用户必须点 button 才能执行删除，这是对的。

9. **配置版本兼容性**：用户从 v0.1 升级到 v0.2（多了 `jpeg_quality` 字段），老的 config.json 反序列化失败会 panic。必须给所有新字段加 `#[serde(default)]` 或 struct 顶部加 `#[serde(default)]`。**在 spec 里我标注了**，实施时务必落实。

10. **`open_system_settings_privacy` 在 macOS 15+ 有变**：新版 macOS 的 URL scheme 可能从 `x-apple.systempreferences:` 改了。如果打不开就 fallback 到 `open 'System Settings'`。

## 十、产出物

完成 spec-09 后你应该有：

- 配置页全部 6 个子页完整可用（含 spec-02 的 API Keys 和 spec-mvp 的测试）
- 真实可用的开机自启、窗口最小化、存储自动清理
- 清晰的隐私可视化和数据清理能力
- 完整的系统信息和版本信息展示
- 一套可复用的 Settings 组件模式（SectionHeader / FieldGroup / ToggleRow / DataClearRow）

**产品完整度**：到这一步，Corivo 作为一个日常使用的桌面工具已经基本完备。用户能启动、能看、能搜、能配置、能清理、能理解数据去向。剩下的都是"锦上添花"层：系统托盘、onboarding、通知声音等。

下一份 spec 的候选：

- **spec-10-onboarding.md**：首次启动引导流程（欢迎 → 填 API key → 授权 → 第一次捕获）
- **spec-11-system-tray.md**：系统托盘 + 菜单 + 快捷操作
- **spec-12-polish.md**：各种视觉打磨和交互细节（加载状态、错误恢复、无障碍）

**我建议下一份做 spec-10 onboarding**。理由：

1. 你内测阶段要找朋友装，**没有 onboarding 朋友打开就会懵**——不知道要先填 API key、不知道要授权屏幕录制
2. onboarding 能把 spec-02/09 里分散的配置项串成一个引导流程，用户体验提升巨大
3. 工程量不大，核心逻辑复用现有组件
4. 有了 onboarding，你的第一次用户反馈才有意义——否则他们反馈的大部分是"找不到怎么开始"而不是"推送内容怎么样"

系统托盘（spec-11）和 polish（spec-12）可以往后放。

---

**回复**：

1. spec-09 有没有想改的地方？特别是 GeneralSection 的分组结构、StorageSection 的空间可视化、PrivacySection 的措辞（"你的数据"这类隐私文案很敏感，改一版你觉得合适的）。
2. 下一份做 spec-10 onboarding 吗？还是你想先做别的？