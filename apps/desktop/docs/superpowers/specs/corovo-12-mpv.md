spec-mvp-loop.md — 最小闭环 Demo
一、目标
一句话：跑通从"截图"到"用户收到一条有价值的系统通知"的完整链路。
完整链路：
定时截图(15s) → 攒够一批(可配) → Gemini 总结 → 写入 Supermemory
→ 拿总结内容搜索 Supermemory 找相关记忆
→ 把"当前总结 + 相关记忆"过一个 prompt 判断是否值得推送
→ 值得 → 发系统通知
→ 不值得 → 静默跳过
同时：在配置页加一个测试通知按钮。
二、不做什么

❌ 不做 SQLite（直接用内存 + 文件系统，够用）
❌ 不做 session / segment 管理（只有一个无限运行的 loop）
❌ 不做时间线 UI（概览页先不变）
❌ 不做记忆列表 UI（先不展示）
❌ 不做截图浏览 / 历史回看
❌ 不做优雅的错误恢复（出错就 log + 跳过）
❌ 不做 pHash 去重
❌ 不做上下文连贯（不带前一段总结）
❌ 不做推送反馈收集

这些全部是后续 spec 的事。MVP 只证明"这条链路能跑通、推送有价值"。
三、成功标准

启动 app，在连接页点「开始捕获」
后台每 15 秒截一张屏幕，每 N 张（默认 5 张，即约 75 秒一轮）触发一次处理
处理流程自动完成：Gemini 总结 → 写 Supermemory → 搜相关 → 判断推送
你正常工作 10 分钟后，至少收到 1 条系统通知（如果有相关记忆被命中）
通知内容不是垃圾——你看完觉得"这个确实和我之前做的事有关联"
在配置页的测试区域点按钮能立刻弹出一条测试通知
整个过程 CPU 占用可接受，没有明显卡顿

四、技术方案
4.1 新增依赖
toml# src-tauri/Cargo.toml 追加
xcap = "0.0.14"
image = "0.25"
tauri-plugin-notification = "2"
前端：
bashpnpm add @tauri-apps/plugin-notification
4.2 capabilities 配置
src-tauri/capabilities/default.json 的 permissions 里追加：
json"notification:default"
4.3 核心模块：CaptureLoop
src-tauri/src/services/capture_loop.rs：
rustuse crate::error::{CorivoError, Result};
use crate::providers::llm::ImageInput;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use xcap::Monitor;

pub struct CaptureConfig {
/// 截图间隔（秒）
pub interval_secs: u64,
/// 每几张截图触发一次处理
pub batch_size: usize,
/// 截图保存目录
pub capture_dir: PathBuf,
/// JPEG 质量 (1-100)
pub jpeg_quality: u8,
}

impl Default for CaptureConfig {
fn default() -> Self {
Self {
interval_secs: 15,
batch_size: 5,
capture_dir: PathBuf::from("captures"),
jpeg_quality: 75,
}
}
}

pub struct CaptureLoop {
running: Arc<AtomicBool>,
config: Arc<Mutex<CaptureConfig>>,
}

impl CaptureLoop {
pub fn new(config: CaptureConfig) -> Self {
Self {
running: Arc::new(AtomicBool::new(false)),
config: Arc::new(Mutex::new(config)),
}
}

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 启动后台截图循环，每攒够一批就通过 callback 把图片数据交出去
    pub async fn start<F, Fut>(
        &self,
        on_batch_ready: F,
    ) where
        F: Fn(Vec<ImageInput>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        if self.running.swap(true, Ordering::SeqCst) {
            tracing::warn!("CaptureLoop already running");
            return;
        }

        let running = self.running.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            let mut batch: Vec<ImageInput> = Vec::new();

            while running.load(Ordering::SeqCst) {
                let cfg = config.lock().await;
                let interval = cfg.interval_secs;
                let batch_size = cfg.batch_size;
                let quality = cfg.jpeg_quality;
                drop(cfg);

                // 截图
                match Self::take_screenshot(quality) {
                    Ok(image_data) => {
                        batch.push(image_data);
                        tracing::debug!("screenshot captured, batch {}/{}", batch.len(), batch_size);

                        if batch.len() >= batch_size {
                            let ready_batch = std::mem::take(&mut batch);
                            tracing::info!("batch ready, {} images, processing...", ready_batch.len());
                            on_batch_ready(ready_batch).await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("screenshot failed: {:?}", e);
                    }
                }

                sleep(Duration::from_secs(interval)).await;
            }

            tracing::info!("CaptureLoop stopped");
        });
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub async fn update_config(&self, interval_secs: u64, batch_size: usize) {
        let mut cfg = self.config.lock().await;
        cfg.interval_secs = interval_secs;
        cfg.batch_size = batch_size;
        tracing::info!("CaptureLoop config updated: {}s interval, {} batch", interval_secs, batch_size);
    }

    fn take_screenshot(jpeg_quality: u8) -> Result<ImageInput> {
        let monitors = Monitor::all()
            .map_err(|e| CorivoError::Internal(format!("获取显示器失败: {}", e)))?;
        let monitor = monitors
            .into_iter()
            .next()
            .ok_or_else(|| CorivoError::Internal("没有可用显示器".to_string()))?;

        let capture = monitor
            .capture_image()
            .map_err(|e| CorivoError::Internal(format!("截图失败: {}", e)))?;

        // RgbaImage → JPEG bytes
        let mut jpeg_buf = std::io::Cursor::new(Vec::new());
        let rgb_image = image::DynamicImage::ImageRgba8(capture).to_rgb8();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg_buf,
            jpeg_quality,
        );
        encoder
            .encode(
                rgb_image.as_raw(),
                rgb_image.width(),
                rgb_image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| CorivoError::Internal(format!("JPEG 编码失败: {}", e)))?;

        Ok(ImageInput::jpeg(jpeg_buf.into_inner()))
    }
}
4.4 核心模块：MvpPipeline
这是整个 MVP 的灵魂。一个函数把 Gemini 总结 → Supermemory 写入 → 搜索相关 → 判断推送 → 发通知 全部串起来。
src-tauri/src/services/mvp_pipeline.rs：
rustuse crate::providers::llm::ImageInput;
use crate::services::llm_service::LlmService;
use crate::services::memory_service::MemoryService;
use crate::providers::memory::{MemoryInput, MemorySource, SearchQuery};
use chrono::Utc;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// 总结 prompt
const SUMMARY_PROMPT: &str = r#"以下是用户最近约 1 分钟内的屏幕截图（按时间顺序）。
请用 2-3 句中文总结用户在做什么，要具体到正在操作的应用、查看的内容、做的事情。
不要说"用户在使用电脑"这种废话。"#;

/// 推送判断 prompt
const PUSH_JUDGMENT_PROMPT: &str = r#"你是一个智能助手，负责判断是否应该主动通知用户。

【用户当前正在做的事】
{current_summary}

【从记忆库中找到的相关历史记录】
{related_memories}

请判断这些历史记录对用户当前的工作是否有价值。判断标准：
1. 历史记录里是否包含用户可能忘记的承诺、截止日期、或待办事项？
2. 历史记录里是否有和当前任务直接相关的上下文，能帮用户节省时间？
3. 历史记录里是否揭示了潜在冲突（时间冲突、决策冲突）？

如果满足以上任意一条，输出 JSON：
{{"should_push": true, "title": "通知标题（10字以内）", "body": "通知正文（30字以内，说清楚为什么通知）"}}

如果都不满足（历史记录和当前活动无关、或者只是重复信息），输出：
{{"should_push": false}}

只输出 JSON，不要任何解释。"#;

pub struct MvpPipeline {
llm: Arc<LlmService>,
memory: Arc<MemoryService>,
app_handle: AppHandle,
}

impl MvpPipeline {
pub fn new(
llm: Arc<LlmService>,
memory: Arc<MemoryService>,
app_handle: AppHandle,
) -> Self {
Self { llm, memory, app_handle }
}

    /// 完整的 MVP 闭环处理流程
    pub async fn process_batch(&self, images: Vec<ImageInput>) {
        let image_count = images.len();
        tracing::info!("MVP pipeline: processing {} images", image_count);

        // Step 1: Gemini 总结
        let summary = match self
            .llm
            .summarize_images(images, SUMMARY_PROMPT.to_string())
            .await
        {
            Ok(resp) => {
                tracing::info!(
                    "Gemini summary: {} (tokens: in={} out={}, cost=${:.4})",
                    &resp.text[..resp.text.len().min(80)],
                    resp.input_tokens,
                    resp.output_tokens,
                    resp.cost_usd
                );
                resp.text
            }
            Err(e) => {
                tracing::error!("Gemini 总结失败: {:?}", e);
                return;
            }
        };

        // Step 2: 写入 Supermemory
        let memory_input = MemoryInput {
            content: summary.clone(),
            source: MemorySource::Screenshot {
                session_id: format!("mvp-{}", Utc::now().format("%Y%m%d")),
                segment_id: Utc::now().timestamp(),
            },
            tags: vec!["screenshot".to_string(), "auto".to_string()],
            occurred_at: Utc::now(),
            metadata: serde_json::json!({
                "image_count": image_count,
                "pipeline": "mvp"
            }),
        };

        match self.memory.add(memory_input).await {
            Ok(id) => tracing::info!("写入 Supermemory 成功: {}", id),
            Err(e) => {
                tracing::error!("写入 Supermemory 失败: {:?}", e);
                // 写入失败不阻断后续流程，继续尝试搜索和推送
            }
        }

        // Step 3: 用当前总结内容搜索 Supermemory 找相关记忆
        let related = match self
            .memory
            .search(SearchQuery {
                query: summary.clone(),
                limit: 5,
                source_kind: None,
            })
            .await
        {
            Ok(results) => {
                tracing::info!("搜索到 {} 条相关记忆", results.len());
                results
            }
            Err(e) => {
                tracing::error!("搜索 Supermemory 失败: {:?}", e);
                return;
            }
        };

        // 如果没有任何相关记忆（刚开始用，库里没东西），跳过推送判断
        if related.is_empty() {
            tracing::info!("没有相关记忆，跳过推送判断");
            return;
        }

        // 过滤掉刚刚写入的那条（避免自己匹配自己）
        let related_filtered: Vec<_> = related
            .into_iter()
            .filter(|m| {
                // 简单策略：过滤掉内容完全相同的
                m.content != summary
            })
            .collect();

        if related_filtered.is_empty() {
            tracing::info!("过滤后没有相关记忆，跳过推送判断");
            return;
        }

        // 拼接相关记忆文本
        let related_text = related_filtered
            .iter()
            .enumerate()
            .map(|(i, m)| {
                format!(
                    "{}. [{}] {}",
                    i + 1,
                    m.occurred_at.format("%m-%d %H:%M"),
                    m.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Step 4: 判断是否值得推送
        let judgment_prompt = PUSH_JUDGMENT_PROMPT
            .replace("{current_summary}", &summary)
            .replace("{related_memories}", &related_text);

        // 用 extract_structured 做判断（不需要图片，但 trait 要求传图片）
        // 这里变通：用 summarize_images 传空图片列表会报错，
        // 所以直接用一个小 hack：构造一个最小的占位图片
        // 更好的方式：给 LlmProvider 加一个 text_only 方法，但 MVP 不值得改 trait
        let judgment_result = match self
            .llm
            .summarize_images(vec![], judgment_prompt.clone())
            .await
        {
            Ok(resp) => resp.text,
            Err(_) => {
                // summarize_images 可能不接受空图片，fallback 到另一种方式
                // 直接调 reqwest 做一次纯文本调用
                match self.text_only_gemini_call(&judgment_prompt).await {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::error!("推送判断调用失败: {:?}", e);
                        return;
                    }
                }
            }
        };

        // Step 5: 解析判断结果并推送
        self.handle_judgment(&judgment_result).await;
    }

    /// 纯文本 Gemini 调用（不带图片），用于推送判断
    async fn text_only_gemini_call(&self, prompt: &str) -> crate::error::Result<String> {
        // 直接复用 LlmService，传一张 1x1 的空白 JPEG 作为占位
        // 这是 MVP 的 hack，后续 spec 会给 LlmProvider 加 text_complete 方法
        let tiny_jpeg = Self::make_tiny_jpeg();
        let resp = self
            .llm
            .summarize_images(vec![ImageInput::jpeg(tiny_jpeg)], prompt.to_string())
            .await?;
        Ok(resp.text)
    }

    fn make_tiny_jpeg() -> Vec<u8> {
        // 创建一个 1x1 白色 JPEG
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255u8, 255, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    }

    async fn handle_judgment(&self, raw: &str) {
        // 容错解析：strip markdown 包裹
        let cleaned = raw
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        #[derive(serde::Deserialize)]
        struct Judgment {
            should_push: bool,
            title: Option<String>,
            body: Option<String>,
        }

        match serde_json::from_str::<Judgment>(cleaned) {
            Ok(j) if j.should_push => {
                let title = j.title.unwrap_or_else(|| "Corivo".to_string());
                let body = j.body.unwrap_or_else(|| "发现可能有用的信息".to_string());
                tracing::info!("推送决策: YES - {} / {}", title, body);
                self.send_notification(&title, &body);
            }
            Ok(_) => {
                tracing::info!("推送决策: NO");
            }
            Err(e) => {
                tracing::warn!("推送判断返回非法 JSON: {} (raw: {})", e, cleaned);
            }
        }
    }

    fn send_notification(&self, title: &str, body: &str) {
        if let Err(e) = self
            .app_handle
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::error!("发送通知失败: {:?}", e);
        } else {
            tracing::info!("通知已发送: {} - {}", title, body);
        }
    }
}
关键设计点：

纯文本 Gemini 调用的 hack：MVP 里推送判断不需要图片，但 LlmProvider 的 summarize_images 要求至少一张图。这里用 1x1 白色 JPEG 占位。hack 归 hack，闭环能跑。后续 spec 给 trait 加 text_complete 方法即可消除。
"自己匹配自己"的过滤：刚写入的总结和搜索用的 query 是同一段话，Supermemory 一定会返回它自身作为最相关结果。用内容精确匹配过滤掉。
错误不阻断：任何一步失败都 log + 跳过，不影响下一轮。MVP 重在跑通。

4.5 Tauri Commands 和装配
src-tauri/src/commands/capture.rs（新增）：
rustuse crate::services::capture_loop::CaptureLoop;
use std::sync::Arc;
use tauri::State;

pub struct CaptureAppState {
pub capture_loop: Arc<CaptureLoop>,
}

#[tauri::command]
pub async fn start_capture(state: State<'_, CaptureAppState>) -> Result<(), String> {
if state.capture_loop.is_running() {
return Err("已经在捕获中".to_string());
}
// 注意：实际的 on_batch_ready callback 需要在 lib.rs setup 里闭包捕获 pipeline
// 这里只是启动信号，具体见 lib.rs 装配代码
Ok(())
}

#[tauri::command]
pub async fn stop_capture(state: State<'_, CaptureAppState>) -> Result<(), String> {
state.capture_loop.stop();
Ok(())
}

#[tauri::command]
pub async fn get_capture_status(state: State<'_, CaptureAppState>) -> Result<bool, String> {
Ok(state.capture_loop.is_running())
}

#[tauri::command]
pub async fn update_capture_config(
state: State<'_, CaptureAppState>,
interval_secs: u64,
batch_size: usize,
) -> Result<(), String> {
state.capture_loop.update_config(interval_secs, batch_size).await;
Ok(())
}
src-tauri/src/commands/notification.rs（新增，用于测试通知）：
rustuse tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
pub async fn send_test_notification(app: AppHandle) -> Result<(), String> {
app.notification()
.builder()
.title("Corivo 测试")
.body("如果你看到这条通知，说明通知功能正常工作！")
.show()
.map_err(|e| format!("发送通知失败: {}", e))
}
lib.rs 装配（关键：把所有模块串起来）：
rust// 在 setup 里追加

use services::capture_loop::{CaptureConfig, CaptureLoop};
use services::mvp_pipeline::MvpPipeline;
use commands::capture::CaptureAppState;

// ... 已有的 config_service, keychain_service, memory_service, llm_service 初始化

let capture_config = CaptureConfig {
interval_secs: config_service.get().capture.interval_secs,
batch_size: config_service.get().capture.segment_duration_mins as usize,
// MVP 把 segment_duration_mins 临时复用为 batch_size
// 默认 15 → 15 张一批 → 约 225 秒一轮
// 你可以改成 5 → 5 张一批 → 约 75 秒一轮（更快看到效果）
capture_dir: app.path().app_data_dir().unwrap().join("captures"),
jpeg_quality: 75,
};

let capture_loop = Arc::new(CaptureLoop::new(capture_config));
app.manage(CaptureAppState {
capture_loop: capture_loop.clone(),
});

// 构造 pipeline
let pipeline = Arc::new(MvpPipeline::new(
llm_service.clone(),
memory_service.clone(),
app.handle().clone(),
));

// 注册 notification plugin
// 注意：要在 Builder 上加 .plugin(tauri_plugin_notification::init())

// invoke_handler 增加
commands::capture::start_capture,
commands::capture::stop_capture,
commands::capture::get_capture_status,
commands::capture::update_capture_config,
commands::notification::send_test_notification,
start_capture 的完整实现（由于 closure 需要捕获 pipeline，必须在 lib.rs 层面做）：
实际的 start_capture command 不能直接调 capture_loop.start() 因为需要 pipeline。改成在 command 里通过 State 拿到两个依赖再组装：
rust// 修改 CaptureAppState 包含 pipeline
pub struct CaptureAppState {
pub capture_loop: Arc<CaptureLoop>,
pub pipeline: Arc<MvpPipeline>,
}

#[tauri::command]
pub async fn start_capture(state: State<'_, CaptureAppState>) -> Result<(), String> {
if state.capture_loop.is_running() {
return Err("已经在捕获中".to_string());
}
let pipeline = state.pipeline.clone();
state.capture_loop.start(move |batch| {
let p = pipeline.clone();
async move {
p.process_batch(batch).await;
}
}).await;
Ok(())
}
4.6 前端改动
tauri.ts 追加：
tsexport async function startCapture(): Promise<void> {
return invoke<void>('start_capture')
}

export async function stopCapture(): Promise<void> {
return invoke<void>('stop_capture')
}

export async function getCaptureStatus(): Promise<boolean> {
return invoke<boolean>('get_capture_status')
}

export async function updateCaptureConfig(
intervalSecs: number,
batchSize: number
): Promise<void> {
return invoke<void>('update_capture_config', { intervalSecs, batchSize })
}

export async function sendTestNotification(): Promise<void> {
return invoke<void>('send_test_notification')
}
连接页：截图连接器最小 UI
把 src/pages/connections/connections-page.tsx 从占位符改成实际可用：
tsximport { useState, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Camera, Loader2, Play, Square } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card } from '@/components/ui/card'
import {
startCapture,
stopCapture,
getCaptureStatus,
updateCaptureConfig,
} from '@/lib/tauri'

export function ConnectionsPage() {
const queryClient = useQueryClient()
const [interval, setInterval] = useState(15)
const [batchSize, setBatchSize] = useState(5)

const { data: isRunning } = useQuery({
queryKey: ['capture-status'],
queryFn: getCaptureStatus,
refetchInterval: 2000,
})

const startMut = useMutation({
mutationFn: startCapture,
onSuccess: () => {
toast.success('截图捕获已启动')
queryClient.invalidateQueries({ queryKey: ['capture-status'] })
},
onError: (e: string) => toast.error(e),
})

const stopMut = useMutation({
mutationFn: stopCapture,
onSuccess: () => {
toast.success('截图捕获已停止')
queryClient.invalidateQueries({ queryKey: ['capture-status'] })
},
onError: (e: string) => toast.error(e),
})

const updateMut = useMutation({
mutationFn: () => updateCaptureConfig(interval, batchSize),
onSuccess: () => toast.success('配置已更新'),
onError: (e: string) => toast.error(e),
})

return (
<div className="space-y-6">
<h1 className="text-xl font-semibold">连接</h1>

      <Card className="p-5 space-y-4">
        <div className="flex items-center gap-3">
          <Camera className="w-5 h-5 text-muted-foreground" />
          <div className="flex-1">
            <div className="text-sm font-medium">屏幕截图</div>
            <div className="text-xs text-muted-foreground">
              {isRunning ? '捕获中...' : '未启动'}
            </div>
          </div>
          {isRunning ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => stopMut.mutate()}
              disabled={stopMut.isPending}
            >
              {stopMut.isPending ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <Square className="w-3 h-3" />
              )}
              <span className="ml-1">停止</span>
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={() => startMut.mutate()}
              disabled={startMut.isPending}
            >
              {startMut.isPending ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <Play className="w-3 h-3" />
              )}
              <span className="ml-1">开始</span>
            </Button>
          )}
        </div>

        <div className="border-t border-border pt-4 grid grid-cols-2 gap-4">
          <div className="space-y-1">
            <Label className="text-xs">截图间隔（秒）</Label>
            <Input
              type="number"
              min={5}
              max={120}
              value={interval}
              onChange={(e) => setInterval(Number(e.target.value))}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs">每批张数</Label>
            <Input
              type="number"
              min={2}
              max={30}
              value={batchSize}
              onChange={(e) => setBatchSize(Number(e.target.value))}
            />
          </div>
        </div>

        <Button
          variant="outline"
          size="sm"
          onClick={() => updateMut.mutate()}
          disabled={updateMut.isPending}
        >
          更新配置
        </Button>

        <div className="text-xs text-muted-foreground">
          每 {interval}s 截一张，攒满 {batchSize} 张后触发 Gemini 总结 → Supermemory 写入 → 相关记忆搜索 → 推送判断
        </div>
      </Card>
    </div>
)
}
配置页追加"测试通知"区域：
在 src/pages/settings/settings-page.tsx 的 section 列表里加一项：
tsxconst sections: { id: Section; label: string }[] = [
{ id: 'general', label: '常规' },
{ id: 'api-keys', label: 'API Keys' },
{ id: 'storage', label: '存储' },
{ id: 'privacy', label: '隐私' },
{ id: 'test', label: '测试' },     // 新增
{ id: 'about', label: '关于' },
]
新建 src/pages/settings/sections/test-section.tsx：
tsximport { useState } from 'react'
import { toast } from 'sonner'
import { Bell, Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { sendTestNotification } from '@/lib/tauri'

export function TestSection() {
const [loading, setLoading] = useState(false)

const handleTest = async () => {
setLoading(true)
try {
await sendTestNotification()
toast.success('通知已发送，检查系统通知中心')
} catch (e) {
toast.error(`发送失败: ${e}`)
} finally {
setLoading(false)
}
}

return (
<div className="space-y-6 max-w-xl">
<div>
<h2 className="text-base font-medium mb-1">测试工具</h2>
<p className="text-xs text-muted-foreground">
验证各项功能是否正常工作
</p>
</div>

      <div className="space-y-2">
        <div className="text-sm font-medium">系统通知</div>
        <p className="text-xs text-muted-foreground">
          点击后会发送一条测试通知到系统通知中心，验证 Corivo 的通知权限是否正确配置。
        </p>
        <Button variant="outline" onClick={handleTest} disabled={loading}>
          {loading ? (
            <Loader2 className="w-3 h-3 mr-2 animate-spin" />
          ) : (
            <Bell className="w-3 h-3 mr-2" />
          )}
          发送测试通知
        </Button>
      </div>
    </div>
)
}
在 settings-page.tsx 里引入并渲染：
tsximport { TestSection } from './sections/test-section'

// 在 render 里
{active === 'test' && <TestSection />}
4.7 Sidebar 的状态指示器接入
更新 src/components/layout/status-indicator.tsx 连接真实状态：
tsximport { useQuery } from '@tanstack/react-query'
import { getCaptureStatus } from '@/lib/tauri'

export function StatusIndicator() {
const { data: isRunning } = useQuery({
queryKey: ['capture-status'],
queryFn: getCaptureStatus,
refetchInterval: 3000,
})

return (
<div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
<span
className={`w-2 h-2 rounded-full ${
          isRunning ? 'bg-green-500' : 'bg-muted-foreground/40'
        }`}
/>
<span>{isRunning ? '捕获中' : '未启动'}</span>
</div>
)
}
五、任务分解
会话范围完成判定1CaptureLoop 实现 + xcap 集成cargo build 通过，手动测试能截图2MvpPipeline 实现（全部 5 步）cargo build 通过3commands/capture.rs + commands/notification.rs + lib.rs 装配（notification plugin 注册）pnpm tauri dev 启动无报错4前端连接页 + 配置页测试区域 + status indicator + tauri.ts 封装UI 能看到按钮并点击5端到端测试：点开始 → 等 2 分钟 → 检查日志和通知闭环跑通
总计预估 2-3 天。 会话 5 是你自己手动测试和调 prompt 的时间，不是让 Claude Code 做的。
六、验收清单

配置页「测试」子项的「发送测试通知」按钮能弹出系统通知
macOS 第一次会弹通知权限申请 → 授权后通知正常
连接页看到截图连接器卡片，点「开始」后状态变为「捕获中」
左下角 StatusIndicator 同步变绿
改截图间隔和批次大小后点「更新配置」生效
终端日志能看到每 15 秒一次 screenshot captured
每攒满一批后看到 batch ready, N images, processing...
看到 Gemini summary: ... 日志输出合理总结
看到 写入 Supermemory 成功: xxx
看到 搜索到 N 条相关记忆
当确实有相关记忆时，看到 推送决策: YES 并收到系统通知
当没有相关记忆（刚开始用）时，看到 没有相关记忆，跳过推送判断
点「停止」后截图和处理流程停止
重新点「开始」能恢复运行
连续跑 10 分钟无 crash、无内存泄漏明显增长

七、坑点预警

macOS 屏幕录制权限：第一次用 xcap 截图会触发系统权限弹窗。授权后必须重启 app 才能生效。在 UI 上加一段提示文字："首次使用需要在系统设置中授权屏幕录制权限并重启 Corivo。"
Supermemory 搜索延迟：刚写入的记忆可能需要几秒才能被搜索到。如果"写入 → 立刻搜索"返回空结果，是正常的。MVP 不需要处理这个，因为搜索的目的是找历史记忆，不是找刚写入的那条。
通知权限在 macOS Sequoia 15+ 更严格：需要在 tauri.conf.json 的 bundle.macOS 里确保 identifier 是反向域名格式 com.corivo.app，否则通知权限申请会静默失败。
推送判断 prompt 可能产出非 JSON：Gemini 偶尔会在 JSON 前后加一句话。容错解析里的 trim_start_matches("```json") 能处理大部分情况，但极端 case 可能还会失败。MVP 接受这个不完美，日志里会看到 warning。
MVP 的 text_only hack：用 1x1 JPEG 占位传给 summarize_images 是可行的但 token 会多 258。每轮多花 ~$0.00003，可忽略。
batch 是内存中的：如果 app 在一批未满时被 kill，那批截图丢失。MVP 接受。
不要把 batch_size 设太大：30 张图 × 300KB ≈ 9MB base64 后 ≈ 12MB，逼近 Gemini 单次请求限制。建议 MVP 阶段 batch_size 设 3-5。
推送频率：每 75 秒（15s × 5）可能触发一次推送判断，最坏情况每分钟多一条通知。MVP 不做频率限制，你手动体感觉得太频繁就调大 batch_size。后续 spec 再加 cooldown。

