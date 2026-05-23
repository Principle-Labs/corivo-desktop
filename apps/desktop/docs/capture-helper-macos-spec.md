# macOS Capture Helper · spec

> 状态：v1 设计稿。依赖 [capture-helper-architecture-spec.md](capture-helper-architecture-spec.md) 锁定的 IPC 契约。
> 这份只描述 **macOS 实现**；Windows 实现见 [capture-helper-windows-spec.md](capture-helper-windows-spec.md)。

---

## 1. 技术栈

| 项 | 选择 |
|---|---|
| 语言 | Swift 5.9+ |
| 最低 macOS | **13.0 (Ventura)** —— ScreenCaptureKit 系统音频要求 13.0；AX/Vision/AVFoundation 全部 13.0 ready |
| 架构 | Universal binary (arm64 + x86_64)；CI 在 Apple Silicon 跑 |
| 异步运行时 | Swift Concurrency (`async/await` + `Task`)；CFRunLoop 仅在 AX observer 那一根线程上用 |
| 编码器 | AVFoundation 内置（AAC LC via AudioToolbox） |
| 构建 | SwiftPM + 一个 `Package.swift`（避免 Xcode project 文件 noise） |
| 输出 | Mach-O executable + `.app` bundle（用 helper bundle 才能拿 plist 权限说明） |

**为什么是 .app bundle 而不是裸 binary**：macOS 系统权限弹窗（Microphone / Screen Recording / Accessibility）显示的是 `Info.plist` 里 `CFBundleDisplayName` + `CFBundleIdentifier`，必须有 bundle 才能控制对话框文案，且 Accessibility 是 per-bundle-id 授权。helper bundle 嵌套在主 .app 内：

```
Corivo.app/
  Contents/
    MacOS/
      corivo-app                                      (Tauri 主进程)
      corivo-capture-helper-aarch64-apple-darwin      (Tauri sidecar 入口，是个壳)
    Resources/
      CorivoCaptureHelper.app/                        (真正的 helper bundle)
        Contents/
          MacOS/CorivoCaptureHelper                   (Swift 编译产物)
          Info.plist                                  (权限文案 + bundle id)
```

sidecar 入口壳脚本（10 行 shell，写在 src-tauri/binaries/）的工作只有：`exec "$bundle_path/Contents/MacOS/CorivoCaptureHelper" "$@"`。

bundle id 选 `com.corivo.app.capture-helper`（和主 app `com.corivo.app` 同 prefix，方便统一签名）。

## 2. 项目结构

```
helpers/macos/
  Package.swift
  Sources/
    CorivoCaptureHelper/
      main.swift                       入口：parse args, install signal handlers, start IPC loop
      IPC/
        Codec.swift                    NDJSON line reader / writer
        Protocol.swift                 v1 message structs (Codable)
        Router.swift                   request dispatcher, capability handshake
        EventEmitter.swift             stdout event sink (thread-safe)
        Heartbeat.swift                Timer 5s
      Audio/
        AudioRecorder.swift            session lifecycle 入口
        SystemAudioSource.swift        SCStream 系统音
        MicrophoneSource.swift         AVCaptureSession 麦克
        SegmentWriter.swift            AVAssetWriter 多轨 + rotate
        Manifest.swift                 manifest.json 序列化
      Screen/
        ScreenCapture.swift            SCK 单帧
        DisplayEnumerator.swift
      AX/
        AccessibilityQuery.swift       focused window walk
        SelectionProbe.swift           AXTextMarker 路径
        AXSubscription.swift           AXObserver per pid
        AXThread.swift                 dedicated CFRunLoop thread
      Foreground/
        ForegroundMonitor.swift        NSWorkspace observer
      OCR/
        VisionOCR.swift
      Permissions/
        PermissionStatus.swift         查询 + 触发请求
      Util/
        Logger.swift                   stderr 输出，被 Rust tracing 捕获
        ULID.swift
        OSError.swift                  OSStatus → JSON error mapper
  Tests/
    CorivoCaptureHelperTests/
      ProtocolTests.swift
      SegmentWriterTests.swift
      ...
  Info.plist                           for the .app bundle
  build.sh                             SwiftPM build + lipo + codesign + bundle 组装
```

依赖（全 Apple 系统框架，零第三方）：`ScreenCaptureKit`, `AVFoundation`, `CoreMedia`, `Vision`, `ApplicationServices` (AX), `AppKit` (NSWorkspace), `Foundation`。

## 3. 入口与 IPC 主循环

```swift
// main.swift
import Foundation

// 防误启：必须由 Rust 主进程 spawn
guard let parentPid = ProcessInfo.processInfo.environment["CORIVO_HELPER_PARENT_PID"],
      Int32(parentPid) != nil else {
    FileHandle.standardError.write(Data("refusing to start: missing CORIVO_HELPER_PARENT_PID\n".utf8))
    exit(2)
}

// echo-mode：CI / 协议测试
let echoMode = CommandLine.arguments.contains("--echo-mode")

let router = Router(echoMode: echoMode)
router.installHandlers()

// 协议握手（必须在任何其他消息之前）
EventEmitter.shared.send(Hello(
    helperVersion: HelperVersion.string,
    supportedProtocols: ["v1"],
    capabilities: Capabilities.current(),
    platform: PlatformInfo.current()
))

// 启动 stdin 读循环
Task.detached(priority: .high) {
    await Codec.readLoop(handler: router.dispatch)
}

// 心跳
Heartbeat.start()

// 信号
SignalHandler.installGracefulShutdown { reason in
    AudioRecorder.shutdown()  // best-effort 落盘
    EventEmitter.shared.flush()
    exit(0)
}

RunLoop.main.run()  // AppKit / AX runloop 需要
```

**线程模型**：
- 主线程跑 `RunLoop.main`（NSWorkspace observer / AppKit 必需）
- stdin 读循环跑 `Task` (cooperative)
- AX observer 跑 dedicated `Thread`（自管 CFRunLoop，对应当前 [ax_observer.rs](apps/desktop/src-tauri/src/services/capture_pipeline/event_sources/ax_observer.rs) 的模型）
- 录音回调来自 SCStream / AVCaptureSession 自己的 dispatch queue
- EventEmitter 内部用 serial queue 做 stdout 写入（避免行交错）

## 4. 音频录制

### 4.1 总体设计

一个 session = 一个 `AudioRecorder` 实例。Recorder 持有：
- 0 或 1 个 `SystemAudioSource`（SCStream，仅 audio 输出）
- 0 或 1 个 `MicrophoneSource`（AVCaptureSession）
- 1 个 `SegmentWriter`（持续轮换 AVAssetWriter）

时间基准：所有 sample buffer 用各自的 PTS（CMTime）；SegmentWriter 用 SCStream 第一帧的 PTS 作为 session origin，之后所有 buffer PTS 减去 origin 得到 session-relative 时间。

### 4.2 系统音频：ScreenCaptureKit audio-only

```swift
final class SystemAudioSource: NSObject, SCStreamDelegate, SCStreamOutput {
    private var stream: SCStream?
    private let queue = DispatchQueue(label: "corivo.helper.sysaudio", qos: .userInteractive)
    weak var sink: SegmentWriter?

    func start() async throws {
        let content = try await SCShareableContent.current
        guard let display = content.displays.first else { throw HelperError.noDisplay }

        // audio-only filter：包含主显示器（必填）但 minimumFrameInterval 设很大避免视频帧
        let filter = SCContentFilter(display: display, excludingApplications: [], exceptingWindows: [])

        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true   // 不录 helper 自己产生的声音（实践中 helper 不发声，但保险）
        config.sampleRate = 48000
        config.channelCount = 2
        // 视频部分：设极低帧率 + 1x1 尺寸；SCK 不允许纯音频流，但这样视频开销几乎为零
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 1)  // 1 fps，立刻 drop

        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        // 注意：不订阅 .screen 输出 → 视频帧虽生成但被 SCK 内部 drop，无 CPU 成本
        try await stream.startCapture()
        self.stream = stream
    }

    func stop() async {
        try? await stream?.stopCapture()
        stream = nil
    }

    // SCStreamOutput
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, sampleBuffer.isValid else { return }
        sink?.appendSystemAudio(sampleBuffer)
    }
}
```

**注意点**：
- macOS 13.0 ScreenCaptureKit 强制要求至少订阅一个 display；纯音频流不被支持。workaround：订阅最小的 display + 不挂 video output handler，视频帧被 SCK 内部丢弃，实测 CPU < 1%
- macOS 14.4+ 后可选用 Core Audio Tap 路径走 per-app 录制（v2 加 capability flag `audio_per_app: true`，本 spec v1 不做）

### 4.3 麦克风：AVCaptureSession

```swift
final class MicrophoneSource: NSObject, AVCaptureAudioDataOutputSampleBufferDelegate {
    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "corivo.helper.mic", qos: .userInteractive)
    weak var sink: SegmentWriter?

    func start(deviceID: String?) throws {
        session.beginConfiguration()
        defer { session.commitConfiguration() }

        let device: AVCaptureDevice
        if let deviceID, let d = AVCaptureDevice(uniqueID: deviceID) {
            device = d
        } else {
            guard let d = AVCaptureDevice.default(for: .audio) else { throw HelperError.noMicrophone }
            device = d
        }

        let input = try AVCaptureDeviceInput(device: device)
        guard session.canAddInput(input) else { throw HelperError.cannotAddInput }
        session.addInput(input)

        let output = AVCaptureAudioDataOutput()
        output.setSampleBufferDelegate(self, queue: queue)
        guard session.canAddOutput(output) else { throw HelperError.cannotAddOutput }
        session.addOutput(output)

        session.startRunning()
    }

    func stop() {
        session.stopRunning()
    }

    func captureOutput(_: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from _: AVCaptureConnection) {
        sink?.appendMicrophone(sampleBuffer)
    }
}
```

**设备枚举** (`recording.list_microphones`)：

```swift
let session = AVCaptureDevice.DiscoverySession(
    deviceTypes: [.microphone, .external],     // .external for AirPods etc.
    mediaType: .audio,
    position: .unspecified
)
let devices = session.devices.map {
    MicrophoneDevice(id: $0.uniqueID, name: $0.localizedName, isDefault: $0.uniqueID == AVCaptureDevice.default(for: .audio)?.uniqueID)
}
```

### 4.4 SegmentWriter（多轨 AAC + rotation）

核心责任：
1. 维护当前活跃的 `AVAssetWriter` + 两条 `AVAssetWriterInput`（一个 sys 轨，一个 mic 轨）
2. 每路 `appendXxx` 调用入队到内部 serial queue 排序后 appendSampleBuffer
3. 当当前 segment 时长达到 `segmentSeconds`（默认 10s），原子地切换到下一个 segment
4. 切换语义：旧 writer `markAsFinished` + `finishWriting` 后再 emit `recording.segment_closed`
5. 每个 segment 是合法独立 .m4a，可单独播放、单独转录

```swift
final class SegmentWriter {
    struct Config {
        let outputDir: URL
        let segmentSeconds: Double          // 10
        let captureSystemAudio: Bool
        let captureMicrophone: Bool
        let sessionID: String
    }

    private let config: Config
    private let queue = DispatchQueue(label: "corivo.helper.segment", qos: .userInteractive)
    private var currentWriter: SegmentInstance?
    private var sessionOrigin: CMTime?      // 第一帧的 PTS
    private var nextIndex = 0

    init(config: Config) throws {
        self.config = config
        try FileManager.default.createDirectory(at: config.outputDir, withIntermediateDirectories: true)
        // 崩溃恢复：扫已有 segment-NNNN.m4a，nextIndex 设为已有最大 +1
        nextIndex = Self.scanNextIndex(in: config.outputDir)
    }

    func appendSystemAudio(_ buffer: CMSampleBuffer) {
        queue.async { [weak self] in self?.appendInternal(buffer, track: .system) }
    }

    func appendMicrophone(_ buffer: CMSampleBuffer) {
        queue.async { [weak self] in self?.appendInternal(buffer, track: .mic) }
    }

    private func appendInternal(_ buffer: CMSampleBuffer, track: TrackKind) {
        let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
        if sessionOrigin == nil { sessionOrigin = pts }

        // 是否需要 rotate
        if let writer = currentWriter, writer.elapsedAt(pts: pts) >= config.segmentSeconds {
            rotate(at: pts)
        }
        if currentWriter == nil { startSegment(at: pts) }

        currentWriter?.append(buffer, track: track)
    }

    private func startSegment(at pts: CMTime) {
        let path = config.outputDir.appendingPathComponent(String(format: "segment-%04d.m4a", nextIndex))
        let writer = try? SegmentInstance(
            url: path,
            startPTS: pts,
            captureSystem: config.captureSystemAudio,
            captureMic: config.captureMicrophone
        )
        currentWriter = writer
        nextIndex += 1
    }

    private func rotate(at pts: CMTime) {
        guard let writer = currentWriter else { return }
        currentWriter = nil
        let index = writer.index
        let path = writer.url
        let startedAt = writer.startedAt
        // 异步 finish，不阻塞采集
        Task.detached(priority: .userInitiated) {
            await writer.finish()
            EventEmitter.shared.send(SegmentClosed(
                sessionID: config.sessionID,
                segmentIndex: index,
                path: path.path,
                durationMs: writer.durationMs,
                startedAt: startedAt,
                endedAt: writer.endedAt!,
                sysTrackPresent: config.captureSystemAudio,
                micTrackPresent: config.captureMicrophone
            ))
        }
    }

    func stop() async {
        await withCheckedContinuation { cont in
            queue.async { [weak self] in
                Task {
                    await self?.currentWriter?.finish()
                    self?.currentWriter = nil
                    cont.resume()
                }
            }
        }
        Manifest.write(at: config.outputDir, sessionID: config.sessionID, ...)
    }
}

private final class SegmentInstance {
    let writer: AVAssetWriter
    let sysInput: AVAssetWriterInput?
    let micInput: AVAssetWriterInput?
    // ...

    init(url: URL, startPTS: CMTime, captureSystem: Bool, captureMic: Bool) throws {
        writer = try AVAssetWriter(outputURL: url, fileType: .m4a)

        let aacSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48000,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 128_000,
        ]

        if captureSystem {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: aacSettings)
            input.expectsMediaDataInRealTime = true
            sysInput = input
            writer.add(input)
        } else { sysInput = nil }

        if captureMic {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: aacSettings)
            input.expectsMediaDataInRealTime = true
            micInput = input
            writer.add(input)
        } else { micInput = nil }

        writer.startWriting()
        writer.startSession(atSourceTime: startPTS)
    }

    func append(_ buffer: CMSampleBuffer, track: TrackKind) {
        let input = (track == .system) ? sysInput : micInput
        guard let input, input.isReadyForMoreMediaData else { return }  // backpressure: drop
        input.append(buffer)
    }

    func finish() async {
        sysInput?.markAsFinished()
        micInput?.markAsFinished()
        await writer.finishWriting()
    }
}
```

**多轨在 .m4a 里的形态**：M4A (ISO base media) 容器原生支持多 audio track。AVAssetWriter 加两个 `AVAssetWriterInput(mediaType: .audio, ...)` 就生成两条独立轨道。下游转录 / 播放器可单独提取（`ffmpeg -map 0:a:0` / `0:a:1`）。

**Backpressure**：`isReadyForMoreMediaData == false` 时直接丢帧（不阻塞采集线程）。同时 emit `recording.error{kind:"backpressure",fatal:false}`，让 Rust 主程序可以告警但不崩。

**Session origin 同步**：sys 和 mic 各自有时钟，用 `host time` 锚定（CMSampleBuffer 自带的 hostTimeMachAbsoluteTime），所以两轨在 m4a 里时间对齐。

### 4.5 Recording RPC handler

```swift
extension Router {
    func handleRecordingStart(_ payload: RecordingStartPayload, id: String) async {
        guard AudioRecorder.shared.activeSession == nil else {
            return respond(id: id, error: .resourceBusy("recording session already active"))
        }
        do {
            let session = try await AudioRecorder.shared.start(
                sessionID: payload.sessionId,
                outputDir: URL(fileURLWithPath: payload.outputDir),
                segmentSeconds: payload.segmentSeconds,
                captureSystemAudio: payload.captureSystemAudio,
                captureMicrophone: payload.captureMicrophone,
                microphoneDeviceID: payload.microphoneDeviceId
            )
            respond(id: id, result: ["session_id": session.id, "started_at": session.startedAt])
        } catch let error as HelperError {
            respond(id: id, error: error.toResponseError())
        }
    }
}
```

## 5. 单帧截图

```swift
final class ScreenCapture {
    func capture(displayID: String?, outputPath: String?, format: ImageFormat, quality: Int) async throws -> Screenshot {
        let content = try await SCShareableContent.current
        let display: SCDisplay
        if let id = displayID, let d = content.displays.first(where: { String($0.displayID) == id }) {
            display = d
        } else {
            guard let main = content.displays.first(where: { $0.displayID == CGMainDisplayID() })
                  ?? content.displays.first else { throw HelperError.noDisplay }
            display = main
        }

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let config = SCStreamConfiguration()
        config.width = display.width
        config.height = display.height

        // SCK 14.0+ 提供 SCScreenshotManager 单帧接口；13.0 用 SCStream + 抓一帧
        let cgImage: CGImage
        if #available(macOS 14.0, *) {
            cgImage = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
        } else {
            cgImage = try await Self.captureSingleFrameViaStream(filter: filter, config: config)
        }

        let path = outputPath.map { URL(fileURLWithPath: $0) } ?? Self.tempPath(format: format)
        try Self.encode(cgImage, to: path, format: format, quality: quality)

        return Screenshot(
            path: path.path,
            width: cgImage.width,
            height: cgImage.height,
            capturedAt: Date()
        )
    }
}
```

`SCScreenshotManager.captureImage` 在 14.0+ 极简（一行）。13.0 路径需要短暂 SCStream + screen output handler 拿一帧后立刻 stopCapture，~50ms 总成本。

`screen.list_displays` 直接走 `SCShareableContent.current.displays`，附 `CGGetActiveDisplayList` 的 main display ID 标记。

## 6. Accessibility

### 6.1 文本抽取（focused window walk）

直接移植 [ax_extractor.rs](apps/desktop/src-tauri/src/services/extractor/ax_extractor.rs) 的算法到 Swift：

```swift
final class AccessibilityQuery {
    static func query(pid: pid_t, opts: AxQueryOpts) throws -> AxText {
        guard AXIsProcessTrusted() else { throw HelperError.permissionDenied(.accessibility) }

        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, Float(opts.deadlineMs) / 1000.0)

        var focused: CFTypeRef?
        let err = AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &focused)
        guard err == .success, let window = focused else {
            throw HelperError.osError(code: Int(err.rawValue), detail: "focused window unavailable")
        }

        var buffer = ""
        var roleTags: [RoleTagSpan] = []
        let deadline = Date().addingTimeInterval(Double(opts.deadlineMs) / 1000.0)

        walk(
            element: window as! AXUIElement,
            depth: 0,
            buffer: &buffer,
            roleTags: &roleTags,
            deadline: deadline,
            maxChars: opts.maxChars,
            maxDepth: opts.maxDepth,
            skip: SkipPredicate(opts.skipPredicate)
        )

        return AxText(text: buffer, roleTags: roleTags, elapsedMs: ..., truncated: buffer.count >= opts.maxChars)
    }

    private static func walk(element: AXUIElement, depth: Int, ...) {
        if depth > maxDepth || Date() > deadline || buffer.count >= maxChars { return }
        // role / subrole / description / title / value 抓取，role tag 同 role_tag() in Rust
        // children 递归
    }
}
```

`SkipPredicate` 支持的语义在协议 spec 里限定（只 string match）；adapter 层的复杂判断仍在 Rust 主进程。

### 6.2 选区探测

直接移植 [selection_probe.rs](apps/desktop/src-tauri/src/services/extractor/selection_probe.rs) 的 AXTextMarker 路径，250ms 硬死线。

### 6.3 AXObserver 事件订阅

dedicated thread 跑 CFRunLoop，模型完全照搬 [ax_observer.rs](apps/desktop/src-tauri/src/services/capture_pipeline/event_sources/ax_observer.rs)：

```swift
final class AXThread {
    private var thread: Thread?
    private let cmdQueue = DispatchQueue(label: "corivo.helper.ax-cmd")
    private var pendingCommands: [Command] = []

    enum Command { case setPid(pid_t, [String]), removePid(pid_t), stop }

    func subscribe(pid: pid_t, notifications: [String]) { /* enqueue */ }
    func unsubscribe(pid: pid_t) { /* enqueue */ }

    private func runLoop() {
        Thread.current.name = "corivo.helper.ax-runloop"
        while !shouldStop {
            drainCommands()
            CFRunLoopRunInMode(.defaultMode, 0.1, false)
        }
    }
}
```

callback 函数是 C 函数指针（Swift 里用 `@convention(c)`），refcon 传 `Unmanaged<AXSubscription>.passUnretained(...).toOpaque()`，回调里取出后 emit 到 `EventEmitter`。

**关键纪律**（同 ax_observer.rs 注释）：refcon 指向的对象生命周期必须长于 observer，所以 `AXSubscription` 由 `AXThread` 持有 strong ref，卸载前先 `AXObserverRemoveNotification` 再 release。

## 7. 前台 app 监听

```swift
final class ForegroundMonitor {
    private var observer: NSObjectProtocol?
    private var lastBundle: String?

    func start() {
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self else { return }
            let app = NSWorkspace.shared.frontmostApplication
            let bundleID = app?.bundleIdentifier
            if bundleID == self.lastBundle { return }   // dedup
            self.lastBundle = bundleID

            EventEmitter.shared.send(ForegroundActivated(
                pid: app?.processIdentifier,
                bundleId: bundleID,
                appName: app?.localizedName,
                windowTitle: nil,    // window title 通过 AX 拿，不在这里
                ts: Date()
            ))
        }
    }

    func stop() {
        if let observer { NSWorkspace.shared.notificationCenter.removeObserver(observer) }
    }
}
```

block 在主线程跑（queue: .main）—— 重要的是 EventEmitter 内部用自己的 serial queue 序列化 stdout 写。

`foreground.current` 同步读 `frontmostApplication` 即可。

## 8. Vision OCR

直接移植 [ocr_extractor.rs](apps/desktop/src-tauri/src/services/extractor/ocr_extractor.rs) 的 VNRecognizeTextRequest 调用：

```swift
final class VisionOCR {
    static func run(imagePath: String, languages: [String], correction: Bool) async throws -> OcrResult {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: imagePath)) else {
            throw HelperError.notFound("image not found: \(imagePath)")
        }
        let handler = VNImageRequestHandler(data: data, options: [:])
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.recognitionLanguages = languages
        request.usesLanguageCorrection = correction

        try handler.perform([request])
        let observations = request.results ?? []

        // 对应现有 stitch_observations 算法
        let items = observations.compactMap { obs -> TextObservation? in
            guard let top = obs.topCandidates(1).first else { return nil }
            return TextObservation(
                text: top.string,
                midX: obs.boundingBox.midX,
                midY: obs.boundingBox.midY,
                height: obs.boundingBox.height
            )
        }
        return OcrResult(text: stitch(observations: items), elapsedMs: ...)
    }
}
```

reading-order 拼接算法和 Rust 的 `output_format::stitch_observations` 一致 —— 这个算法本身是 platform-independent 但移植过去更省 IPC 一次。

## 9. 错误转换

Swift error → IPC error 的统一桥：

```swift
enum HelperError: Error {
    case permissionDenied(PermissionKind)
    case notFound(String)
    case resourceBusy(String)
    case unsupported(String)
    case osError(code: Int, detail: String)
    case timeout(TimeInterval)
    case `internal`(String)

    func toResponseError() -> ResponseError {
        switch self {
        case .permissionDenied(let kind):
            return ResponseError(code: "PERMISSION_DENIED", message: "permission denied: \(kind)", detail: ["kind": kind.rawValue])
        // ...
        }
    }
}
```

`OSStatus` 用 `osstatus_to_string` 翻译为可读名（兼容 ax / coreaudio / screencapturekit 各自的错误码空间）。

## 10. 权限处理

### 10.1 状态查询（不弹窗）

```swift
struct PermissionStatus {
    static func current() -> PermissionStatusResult {
        return PermissionStatusResult(
            microphone: queryMicrophone(),
            screenRecording: queryScreenRecording(),
            accessibility: queryAccessibility()
        )
    }

    private static func queryMicrophone() -> Status {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return .granted
        case .denied, .restricted: return .denied
        case .notDetermined: return .undetermined
        @unknown default: return .undetermined
        }
    }

    private static func queryScreenRecording() -> Status {
        // 13.0+ 的 CGPreflightScreenCaptureAccess，无副作用
        return CGPreflightScreenCaptureAccess() ? .granted : .undetermined
    }

    private static func queryAccessibility() -> Status {
        // AXIsProcessTrustedWithOptions(nil) 不弹窗只查询
        return AXIsProcessTrusted() ? .granted : .undetermined
    }
}
```

### 10.2 触发授权

```swift
extension Router {
    func handlePermissionRequest(kind: PermissionKind, id: String) async {
        switch kind {
        case .microphone:
            // 真实弹窗：第一次走任意 AVCaptureSession 即触发；这里手动驱动
            let granted = await AVCaptureDevice.requestAccess(for: .audio)
            respond(id: id, result: ["granted": granted])
        case .screenRecording:
            // 没有 silent request；只能请用户去系统设置
            CGRequestScreenCaptureAccess()  // 14.0+ 触发 prompt；< 14 直接打开 System Settings
            respond(id: id, result: ["opened_system_settings": true])
        case .accessibility:
            // 同样无 silent request；触发系统弹窗
            let opts = ["AXTrustedCheckOptionPrompt": true] as CFDictionary
            _ = AXIsProcessTrustedWithOptions(opts)
            respond(id: id, result: ["opened_system_prompt": true])
        }
    }
}
```

UI 引导文案在 Rust onboarding 页面写。

## 11. Info.plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.corivo.app.capture-helper</string>
  <key>CFBundleName</key>
  <string>CorivoCaptureHelper</string>
  <key>CFBundleDisplayName</key>
  <string>Corivo Capture</string>
  <key>CFBundleExecutable</key>
  <string>CorivoCaptureHelper</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>LSUIElement</key>
  <true/>  <!-- 不在 Dock / 不接受输入焦点 -->
  <key>LSBackgroundOnly</key>
  <true/>  <!-- 完全后台 -->
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>

  <!-- 权限文案 -->
  <key>NSMicrophoneUsageDescription</key>
  <string>Corivo 需要麦克风权限以录制会议音频</string>
  <!-- ScreenRecording 通过 hardened runtime entitlement 控制（见 12 节） -->
</dict>
</plist>
```

## 12. 签名 & 公证

| 项 | 配置 |
|---|---|
| Code signing identity | 与主 app 同一个 Developer ID Application |
| Hardened runtime | 必须开启 |
| Entitlements file | `helpers/macos/CorivoCaptureHelper.entitlements` |
| Notarization | 主 app 公证流程把 helper bundle 一起 notarize（嵌套签名） |

`CorivoCaptureHelper.entitlements`：

```xml
<plist version="1.0">
<dict>
  <key>com.apple.security.device.audio-input</key>
  <true/>
  <key>com.apple.security.device.camera</key>
  <false/>  <!-- 不需要摄像头 -->
  <key>com.apple.security.cs.disable-library-validation</key>
  <false/>  <!-- 严格验证：不 dlopen 第三方 dylib -->
</dict>
</plist>
```

**重要**：helper 的 Team ID 必须和主 app 一致，否则 Tauri 主进程 spawn helper 时会触发 SIP / Gatekeeper 拒绝。

构建脚本 `helpers/macos/build.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# 1. SwiftPM 编译 universal binary
swift build -c release --arch arm64
swift build -c release --arch x86_64
mkdir -p .build/universal
lipo -create -output .build/universal/CorivoCaptureHelper \
  .build/arm64-apple-macosx/release/CorivoCaptureHelper \
  .build/x86_64-apple-macosx/release/CorivoCaptureHelper

# 2. 组装 .app bundle
APP=.build/CorivoCaptureHelper.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp .build/universal/CorivoCaptureHelper "$APP/Contents/MacOS/"
cp Info.plist "$APP/Contents/"

# 3. 签名
codesign --force --options runtime \
  --entitlements CorivoCaptureHelper.entitlements \
  --sign "$DEV_ID_IDENTITY" \
  "$APP"

# 4. 拷到 src-tauri/binaries/，加 target triple 后缀给 Tauri sidecar 命名约定识别
DEST="../../src-tauri/binaries"
mkdir -p "$DEST/CorivoCaptureHelper.app"
ditto "$APP" "$DEST/CorivoCaptureHelper.app"
# sidecar entry 壳脚本（Tauri 找的是这个文件名）
cat > "$DEST/corivo-capture-helper-aarch64-apple-darwin" <<'EOF'
#!/bin/bash
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$DIR/CorivoCaptureHelper.app/Contents/MacOS/CorivoCaptureHelper" "$@"
EOF
chmod +x "$DEST/corivo-capture-helper-aarch64-apple-darwin"
cp "$DEST/corivo-capture-helper-aarch64-apple-darwin" "$DEST/corivo-capture-helper-x86_64-apple-darwin"
```

`pnpm tauri:build` 之前 `turbo.json` 里 desktop 包的 `tauri:build` 任务依赖一个 `build:helper-macos` 任务调起这个脚本。

## 13. 测试

### 13.1 单元

XCTest，每个核心模块至少：
- `ProtocolTests`：v1 schema 序列化 round-trip + 边界 case
- `SegmentWriterTests`：用 fixture CMSampleBuffer 序列模拟 sys+mic 流，验证 segment 切分时序
- `AccessibilityQueryTests`：AX walk 算法的纯逻辑（用一个 mock AXUIElement 实现）
- `RoleTagTests`：role → tag 映射

### 13.2 端到端

`TestPlan-Live.xctestplan` 里的 case，需要在真 Mac 跑（CI 用 macOS runner）：
- 录 30s session：assert 至少 3 个 segment 文件、能用 `ffprobe` 读出 2 条 audio track
- 截图 main display：assert 文件存在 + 大小合理
- AX query Finder pid：assert 拿到 `[TITLE]` 标记
- 1 小时 soak（不在每次 PR 跑，nightly）：assert 内存增长 < 100 MB、无崩溃、segment 数 ≈ 360

### 13.3 协议合规

`echo-mode` 的契约测试：Rust 端 `tests/capture_helper_protocol.rs` 跑真实 helper binary 的 echo-mode，全协议 message 走一遍 round-trip。这个测试**不需要任何系统能力**（mic / screen / ax 全部 mock 掉），可以在 CI 任意 runner 跑。

## 14. 性能预算

| 操作 | 预算 | 监控 |
|---|---|---|
| `screen.capture` 1 帧 5K | < 200 ms | 每次 emit 内含 elapsed_ms |
| `ax.query` 典型 8KB 输出 | < 500 ms | 同上 |
| `ocr.run` 5K 截图 | < 1500 ms | 同上 |
| 录音稳态 CPU（sys+mic+encoder） | < 8% on M2 | helper 进程 ps -o %cpu |
| 录音稳态内存 | < 80 MB | RSS |
| Segment 切换抖动 | < 10 ms 间隙 | `ffprobe` segment N 末尾 vs N+1 起始 |

不达标 → 回头 profile，不允许"先上线再说"。

## 15. 已知风险 & 留白

| 风险 | 处理 |
|---|---|
| SCK 强制拉视频流（即使丢弃）的 1x1 hack 在某个 macOS 版本被改 | E2E 测里加"录音不订视频时 sys audio 流仍正常"的 assertion；变化时切到 14.4+ 的纯音频 SCStreamConfiguration（如 Apple 加了的话） |
| AVCaptureSession 在 sandbox 里 mic 设备列表为空 | 不进 sandbox（hardened runtime 即可，不开 sandbox） |
| Rosetta 下 helper 跑性能差 | universal binary，CI 验证两个架构下都跑 1 小时 soak |
| AX 在新 macOS 上对某 Electron 构建版本拒绝 | objc_safe 模式的 NSException 捕获在 Swift 里就是 do/catch，已天然安全 |
