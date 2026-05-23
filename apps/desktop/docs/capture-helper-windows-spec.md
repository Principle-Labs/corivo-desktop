# Windows Capture Helper · spec

> 状态：v1 设计稿。依赖 [capture-helper-architecture-spec.md](capture-helper-architecture-spec.md) 锁定的 IPC 契约。
> 这份只描述 **Windows 实现**；macOS 实现见 [capture-helper-macos-spec.md](capture-helper-macos-spec.md)。

---

## 1. 技术栈

| 项 | 选择 |
|---|---|
| 语言 | C++20 |
| 编译器 | MSVC v143（VS 2022 17.8+）—— 受 cppwinrt 模块化要求 |
| WinRT 投影 | **C++/WinRT** (`cppwinrt`) —— 微软官方一等公民，比 windows-rs 快、比 C++/CX 干净 |
| 最低 Windows | **Windows 10 1903 (build 18362)** —— Windows.Graphics.Capture API 要求；WASAPI / UIA / Windows.Media.Ocr 都更早 |
| 异步运行时 | C++/WinRT 的 `IAsyncOperation` + `co_await`（基于 PPL/concurrency runtime） |
| COM 线程模型 | **MTA**（multi-threaded apartment）—— 录音 / 截图 callback 需要 MTA；UIA / WinEvent 在专用 STA worker 线程 |
| 编码器 | Media Foundation 内置 AAC（Win10+ 都有） |
| 构建 | CMake 3.26+ + vcpkg（vcpkg 仅用于 nlohmann/json，其他全 Windows SDK） |
| 输出 | Win32 console subsystem .exe（无窗口） |

**为什么不是 C# + NativeAOT**：NativeAOT 体积（~15 MB 起）、冷启动（~80 ms）、对部分 WinRT API 反射的限制，都比 C++/cppwinrt 差。team 没有现成 .NET infra 时 cppwinrt 是更轻的选项。

**为什么不是 Rust + windows-rs**：见 [架构 spec § 3](capture-helper-architecture-spec.md#3-helper-边界the-contract) Day 1 判定标准；symmetric design 要求 macOS Swift / Windows native — Rust 写 Windows native 等于一半的隔离、双倍的代码复杂度。

## 2. 项目结构

```
helpers/windows/
  CMakeLists.txt
  vcpkg.json                              { "dependencies": ["nlohmann-json"] }
  src/
    main.cpp                              入口
    ipc/
      codec.hpp/.cpp                      NDJSON line io（用 std::cin / std::cout，二进制模式）
      protocol.hpp                        v1 message structs（boost-pfr-style 反射 + nlohmann/json）
      router.hpp/.cpp                     dispatcher
      event_emitter.hpp/.cpp              stdout event sink，serial mutex
      heartbeat.hpp/.cpp
    audio/
      audio_recorder.hpp/.cpp             session lifecycle 入口
      system_audio_source.hpp/.cpp        WASAPI loopback
      microphone_source.hpp/.cpp          WASAPI capture
      segment_writer.hpp/.cpp             Media Foundation Sink Writer 多轨 + rotate
      mf_init.hpp/.cpp                    MFStartup / MFShutdown RAII
      manifest.hpp/.cpp
    screen/
      screen_capture.hpp/.cpp             WGC 单帧
      display_enumerator.hpp/.cpp         EnumDisplayMonitors
      d3d_device.hpp/.cpp                 共享 ID3D11Device for WGC
    ax/
      ui_automation.hpp/.cpp              IUIAutomation 全局实例（MTA）
      accessibility_query.hpp/.cpp        focused window walk
      selection_probe.hpp/.cpp            TextPattern 路径
      ax_subscription.hpp/.cpp            UIA event handlers per-pid
      ax_thread.hpp/.cpp                  专用 STA worker（UIA event thread）
    foreground/
      foreground_monitor.hpp/.cpp         SetWinEventHook + dedicated thread
    ocr/
      windows_ocr.hpp/.cpp                Windows.Media.Ocr.OcrEngine
    permissions/
      permission_status.hpp/.cpp
    util/
      logger.hpp                          stderr 输出
      ulid.hpp
      hresult_to_string.hpp               HRESULT → 可读名
      wide_string.hpp                     UTF-8 ↔ UTF-16 桥
  tests/
    protocol_tests.cpp                    GoogleTest
    segment_writer_tests.cpp
    ...
  CorivoCaptureHelper.manifest            UAC + DPI awareness manifest
  resource.rc                             version info, icon
```

依赖（除 nlohmann/json 外全 Windows SDK 自带）：
- `windowsapp.lib` (cppwinrt)
- `mmdevapi.lib` `audioses.lib` (WASAPI)
- `mfplat.lib` `mfreadwrite.lib` `mfuuid.lib` `mf.lib` (Media Foundation)
- `d3d11.lib` `dxgi.lib` (WGC 需要 D3D device)
- `uiautomationcore.lib` (UIA)
- `oleaut32.lib` `ole32.lib` `propsys.lib` (COM / variants)

## 3. 入口与 IPC 主循环

```cpp
// main.cpp
#include <winrt/base.h>
#include <Windows.h>
#include <iostream>

int wmain(int argc, wchar_t** argv) {
    // 防误启
    auto parent = _wgetenv(L"CORIVO_HELPER_PARENT_PID");
    if (!parent || wcstol(parent, nullptr, 10) <= 0) {
        std::wcerr << L"refusing to start: missing CORIVO_HELPER_PARENT_PID\n";
        return 2;
    }

    // stdout / stdin 切到 binary 模式（防 \r\n 翻译破坏 NDJSON）
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);

    // COM init: MTA（大多数 WinRT API 偏好；UIA event thread 自己再 OleInitialize STA）
    winrt::init_apartment(winrt::apartment_type::multi_threaded);
    auto com_cleanup = wil::scope_exit([]{ winrt::uninit_apartment(); });

    // MF startup
    THROW_IF_FAILED(MFStartup(MF_VERSION, MFSTARTUP_FULL));
    auto mf_cleanup = wil::scope_exit([]{ MFShutdown(); });

    bool echo_mode = false;
    for (int i = 1; i < argc; ++i) {
        if (std::wstring_view(argv[i]) == L"--echo-mode") echo_mode = true;
    }

    Router router{echo_mode};
    router.install_handlers();

    // 协议握手
    EventEmitter::instance().send(make_hello());

    // 心跳
    Heartbeat::start();

    // signal: AttachConsole + SetConsoleCtrlHandler
    SetConsoleCtrlHandler([](DWORD ctrl) -> BOOL {
        if (ctrl == CTRL_C_EVENT || ctrl == CTRL_BREAK_EVENT || ctrl == CTRL_CLOSE_EVENT) {
            graceful_shutdown();
            return TRUE;
        }
        return FALSE;
    }, TRUE);

    // stdin 读循环（同步阻塞，独占主线程）
    Codec::read_loop([&](nlohmann::json msg) { router.dispatch(std::move(msg)); });

    return 0;
}
```

**线程模型**：
- 主线程：MTA + stdin 读循环
- WASAPI capture：每个 source 一根 dedicated thread（render-thread-affinity 设 `AvSetMmThreadCharacteristics(L"Pro Audio", ...)`）
- MF Sink Writer：内部 worker（hardware MFT 自带）
- WGC：D3D11 device 上下文，回调来自 WGC framework 内部线程
- UIA event：专用 STA thread + message pump
- WinEventHook：专用 thread + message pump（ForegroundMonitor 内部）
- EventEmitter：`std::mutex` 包 stdout 写

## 4. 音频录制

### 4.1 总体设计

一个 session = 一个 `AudioRecorder`。Recorder 持有：
- 0 / 1 个 `SystemAudioSource`（WASAPI loopback on default render device）
- 0 / 1 个 `MicrophoneSource`（WASAPI capture on chosen device）
- 1 个 `SegmentWriter`（MF Sink Writer 多轨 + rotation）

时间基准：以系统第一帧的 `IAudioCaptureClient::GetBuffer` 返回的 `u64 device position`（转 100ns 单位）作为 origin；之后每帧 PTS 用 device position 减 origin。

### 4.2 系统音频：WASAPI loopback

```cpp
class SystemAudioSource {
public:
    void start(SegmentWriter& sink) {
        winrt::com_ptr<IMMDeviceEnumerator> enumerator;
        THROW_IF_FAILED(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr,
            CLSCTX_ALL, IID_PPV_ARGS(enumerator.put())));

        THROW_IF_FAILED(enumerator->GetDefaultAudioEndpoint(
            eRender, eConsole, device_.put()));

        THROW_IF_FAILED(device_->Activate(__uuidof(IAudioClient3), CLSCTX_ALL, nullptr,
            reinterpret_cast<void**>(client_.put())));

        WAVEFORMATEX* mix_format = nullptr;
        THROW_IF_FAILED(client_->GetMixFormat(&mix_format));
        format_ = upgrade_to_pcm_float32(*mix_format);  // 强制 float32 PCM 简化下游

        // AUDCLNT_STREAMFLAGS_LOOPBACK ← 系统音的关键
        DWORD flags = AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        REFERENCE_TIME buffer_dur = 200 * 10000;  // 200 ms

        THROW_IF_FAILED(client_->Initialize(
            AUDCLNT_SHAREMODE_SHARED, flags, buffer_dur, 0, &format_, nullptr));

        THROW_IF_FAILED(client_->GetService(IID_PPV_ARGS(capture_.put())));

        sink_ = &sink;
        running_ = true;
        thread_ = std::thread([this]{ capture_loop(); });
        THROW_IF_FAILED(client_->Start());
    }

    void stop() {
        running_ = false;
        if (client_) client_->Stop();
        if (thread_.joinable()) thread_.join();
    }

private:
    void capture_loop() {
        // 提升线程优先级到 Pro Audio
        DWORD task_idx = 0;
        HANDLE mm_handle = AvSetMmThreadCharacteristicsW(L"Pro Audio", &task_idx);
        auto cleanup = wil::scope_exit([&]{ if (mm_handle) AvRevertMmThreadCharacteristics(mm_handle); });

        UINT32 buffer_frame_count = 0;
        client_->GetBufferSize(&buffer_frame_count);

        // poll loop（可换成 event-driven，但 loopback 没有 sample-ready event 支持，poll 是约定方式）
        while (running_) {
            UINT32 packet_size = 0;
            HRESULT hr = capture_->GetNextPacketSize(&packet_size);
            if (FAILED(hr)) { /* report */ break; }

            if (packet_size == 0) {
                std::this_thread::sleep_for(std::chrono::milliseconds(5));  // 匹配 buffer 一半
                continue;
            }

            BYTE* data = nullptr;
            UINT32 frames = 0;
            DWORD flags = 0;
            UINT64 device_pos = 0;
            UINT64 qpc_pos = 0;
            hr = capture_->GetBuffer(&data, &frames, &flags, &device_pos, &qpc_pos);
            if (SUCCEEDED(hr) && frames > 0) {
                bool silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT);
                sink_->append_system_audio(
                    data, frames, format_, qpc_pos, silent
                );
            }
            capture_->ReleaseBuffer(frames);
        }
    }

    winrt::com_ptr<IMMDevice> device_;
    winrt::com_ptr<IAudioClient3> client_;
    winrt::com_ptr<IAudioCaptureClient> capture_;
    WAVEFORMATEX format_{};
    SegmentWriter* sink_ = nullptr;
    std::atomic<bool> running_{false};
    std::thread thread_;
};
```

**为什么 `eConsole` 不是 `eMultimedia`**：会议 app 通常用默认的"沟通"设备，`eConsole` 命中用户当前主输出（耳机/扬声器），与 macOS SCK 行为对齐。

**v2 留口**：Win10 2004+ 提供 process loopback API（`ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_PARAMS`），可以按 PID 录某个 app —— 对应 macOS Core Audio Tap，加 `audio_per_app` capability 时同时上线。

### 4.3 麦克风：WASAPI capture

几乎和 4.2 一样的代码，区别：
- `enumerator->GetDefaultAudioEndpoint(eCapture, eCommunications, ...)`
- 不传 `AUDCLNT_STREAMFLAGS_LOOPBACK`
- `eCommunications` 让我们对齐用户为通话选的 mic

设备枚举（`recording.list_microphones`）：

```cpp
winrt::com_ptr<IMMDeviceCollection> coll;
enumerator->EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE, coll.put());
UINT count = 0; coll->GetCount(&count);

winrt::com_ptr<IMMDevice> default_dev;
enumerator->GetDefaultAudioEndpoint(eCapture, eCommunications, default_dev.put());
LPWSTR default_id_w = nullptr; default_dev->GetId(&default_id_w);
std::wstring default_id = default_id_w; CoTaskMemFree(default_id_w);

std::vector<MicrophoneDevice> devices;
for (UINT i = 0; i < count; ++i) {
    winrt::com_ptr<IMMDevice> dev; coll->Item(i, dev.put());
    LPWSTR id_w = nullptr; dev->GetId(&id_w);
    std::wstring id = id_w; CoTaskMemFree(id_w);

    winrt::com_ptr<IPropertyStore> props;
    dev->OpenPropertyStore(STGM_READ, props.put());
    PROPVARIANT name_var; PropVariantInit(&name_var);
    props->GetValue(PKEY_Device_FriendlyName, &name_var);
    std::wstring name = name_var.pwszVal;
    PropVariantClear(&name_var);

    devices.push_back({
        utf16_to_utf8(id), utf16_to_utf8(name), id == default_id
    });
}
```

### 4.4 SegmentWriter（多轨 AAC + rotation via Media Foundation Sink Writer）

```cpp
class SegmentWriter {
public:
    struct Config {
        std::filesystem::path output_dir;
        double segment_seconds;        // 10
        bool capture_system_audio;
        bool capture_microphone;
        std::string session_id;
    };

    explicit SegmentWriter(Config cfg);

    void append_system_audio(BYTE* pcm, UINT32 frames, WAVEFORMATEX const& fmt,
                             UINT64 qpc_100ns, bool silent);
    void append_microphone(BYTE* pcm, UINT32 frames, WAVEFORMATEX const& fmt,
                           UINT64 qpc_100ns, bool silent);
    void stop();   // 同步等当前 segment finalize 写盘

private:
    void enqueue(TrackKind track, std::vector<BYTE> pcm, UINT64 pts);
    void worker_loop();
    void rotate(UINT64 pts);
    void start_segment(UINT64 pts);
    static UINT32 scan_next_index(std::filesystem::path const& dir);

    Config cfg_;
    std::thread worker_;
    std::mutex mu_;
    std::condition_variable cv_;
    std::queue<WriteTask> queue_;
    std::atomic<bool> running_{true};

    // 当前活跃 segment
    winrt::com_ptr<IMFSinkWriter> writer_;
    DWORD sys_stream_index_ = MF_SINK_WRITER_INVALID_STREAM_INDEX;
    DWORD mic_stream_index_ = MF_SINK_WRITER_INVALID_STREAM_INDEX;
    UINT64 segment_start_pts_ = 0;
    UINT64 session_origin_ = 0;
    UINT32 next_index_ = 0;
};
```

`start_segment` 关键代码：

```cpp
void SegmentWriter::start_segment(UINT64 pts) {
    auto path = cfg_.output_dir / std::format("segment-{:04}.m4a", next_index_);
    THROW_IF_FAILED(MFCreateSinkWriterFromURL(
        path.wstring().c_str(), nullptr, nullptr, writer_.put()));

    auto add_track = [&](DWORD& out_index) {
        winrt::com_ptr<IMFMediaType> output_type;
        THROW_IF_FAILED(MFCreateMediaType(output_type.put()));
        output_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Audio);
        output_type->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_AAC);
        output_type->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
        output_type->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000);
        output_type->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, 2);
        output_type->SetUINT32(MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16000);  // 128 kbps
        output_type->SetUINT32(MF_MT_AUDIO_BLOCK_ALIGNMENT, 1);
        THROW_IF_FAILED(writer_->AddStream(output_type.get(), &out_index));

        // 输入类型：PCM float32（来自 WASAPI），交给 MFT 转码
        winrt::com_ptr<IMFMediaType> input_type;
        THROW_IF_FAILED(MFCreateMediaType(input_type.put()));
        input_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Audio);
        input_type->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_Float);
        input_type->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 32);
        input_type->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000);
        input_type->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, 2);
        THROW_IF_FAILED(writer_->SetInputMediaType(out_index, input_type.get(), nullptr));
    };

    if (cfg_.capture_system_audio)  add_track(sys_stream_index_);
    if (cfg_.capture_microphone)    add_track(mic_stream_index_);

    THROW_IF_FAILED(writer_->BeginWriting());
    segment_start_pts_ = pts;
    next_index_ += 1;
}
```

每条 `append_*`：

```cpp
void SegmentWriter::feed(DWORD stream_idx, BYTE const* pcm, UINT32 bytes,
                          UINT64 pts_relative_to_session) {
    winrt::com_ptr<IMFMediaBuffer> buffer;
    THROW_IF_FAILED(MFCreateMemoryBuffer(bytes, buffer.put()));
    BYTE* dst = nullptr;
    buffer->Lock(&dst, nullptr, nullptr);
    memcpy(dst, pcm, bytes);
    buffer->Unlock();
    buffer->SetCurrentLength(bytes);

    winrt::com_ptr<IMFSample> sample;
    THROW_IF_FAILED(MFCreateSample(sample.put()));
    sample->AddBuffer(buffer.get());
    // PTS 必须是 session-relative 100ns
    sample->SetSampleTime(static_cast<LONGLONG>(pts_relative_to_session));
    sample->SetSampleDuration(static_cast<LONGLONG>(bytes / format_byte_rate_ * 10'000'000));

    HRESULT hr = writer_->WriteSample(stream_idx, sample.get());
    if (FAILED(hr)) {
        // backpressure 不存在于 MF Sink Writer（同步 API），失败一律是真的错
        EventEmitter::instance().send(RecordingError{
            cfg_.session_id, "WRITE_SAMPLE_FAILED", hresult_to_string(hr), false
        });
    }
}
```

**Rotation**：worker 在每次 append 前检查 `pts - segment_start_pts_ >= segment_seconds * 10'000'000`。是 → `writer_->Finalize()` 旧 writer，emit `segment_closed`，`start_segment(pts)` 开新的。Finalize 是同步操作，~50-200ms；为了不阻塞采集线程，整个 worker_loop 本身就在独立线程，rotation 自然异步于采集源。

**多轨 m4a**：MF Sink Writer 的 .m4a / .mp4 容器原生支持多 audio stream，添加多个 stream 即可，无需特殊配置。

### 4.5 编码器可用性兜底

某些 Windows N/KN edition 默认无 Media Feature Pack → 没有 AAC 编码器 → `AddStream` 返回 `MF_E_INVALIDMEDIATYPE`。检测路径：

```cpp
// helper 启动 capability 协商时探测
bool aac_available() {
    winrt::com_ptr<IMFTransform> mft;
    HRESULT hr = CoCreateInstance(
        CLSID_AACMFTEncoder, nullptr, CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(mft.put()));
    return SUCCEEDED(hr);
}
```

不可用 → `capabilities.audio_record = false` + log 警告。Rust 主侧 onboarding 引导用户装 Media Feature Pack。

## 5. 单帧截图（WGC）

```cpp
class ScreenCapture {
public:
    winrt::Windows::Foundation::IAsyncOperation<Screenshot> capture_async(
        std::optional<std::wstring> display_id,
        std::optional<std::filesystem::path> output_path,
        ImageFormat format, int quality)
    {
        auto monitor = display_id.has_value()
            ? find_monitor(*display_id)
            : MonitorFromWindow(nullptr, MONITOR_DEFAULTTOPRIMARY);

        // 创建 GraphicsCaptureItem
        auto interop = winrt::get_activation_factory<
            winrt::Windows::Graphics::Capture::GraphicsCaptureItem,
            IGraphicsCaptureItemInterop>();
        winrt::Windows::Graphics::Capture::GraphicsCaptureItem item{nullptr};
        winrt::check_hresult(interop->CreateForMonitor(monitor,
            winrt::guid_of<winrt::Windows::Graphics::Capture::GraphicsCaptureItem>(),
            winrt::put_abi(item)));

        // D3D device
        auto& d3d = D3DDevice::shared();

        auto frame_pool = winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool::CreateFreeThreaded(
            d3d.winrt_device(),
            winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            item.Size());

        auto session = frame_pool.CreateCaptureSession(item);
        if (winrt::Windows::Foundation::Metadata::ApiInformation::IsPropertyPresent(
            L"Windows.Graphics.Capture.GraphicsCaptureSession", L"IsCursorCaptureEnabled")) {
            session.IsCursorCaptureEnabled(false);  // 截图不带光标
        }

        auto frame_received = std::make_shared<winrt::handle>(
            CreateEvent(nullptr, TRUE, FALSE, nullptr));
        winrt::Windows::Graphics::Capture::Direct3D11CaptureFrame captured_frame{nullptr};

        frame_pool.FrameArrived([&](auto const& sender, auto const&) {
            auto frame = sender.TryGetNextFrame();
            if (frame) {
                captured_frame = frame;
                SetEvent(frame_received->get());
            }
        });

        session.StartCapture();

        // 等单帧 (timeout 2s)
        WaitForSingleObject(frame_received->get(), 2000);
        session.Close();
        frame_pool.Close();
        if (!captured_frame) co_return Screenshot{};  // 错误

        // CPU 拷贝 + 编码
        auto path = output_path.value_or(temp_path(format));
        encode_d3d_texture_to_file(captured_frame.Surface(), path, format, quality);

        co_return Screenshot{
            path.string(),
            captured_frame.ContentSize().Width,
            captured_frame.ContentSize().Height,
            std::chrono::system_clock::now()
        };
    }
};
```

`encode_d3d_texture_to_file` 用 WIC（Windows Imaging Component）：D3D texture → CPU staging texture → IWICBitmap → IWICStream → JPEG/PNG encoder。

`screen.list_displays`: 用 `EnumDisplayMonitors` 枚举，HMONITOR 转字符串作为 ID（用 `GetMonitorInfoW` 拿 device name）。

## 6. UI Automation（accessibility）

### 6.1 全局实例

```cpp
class UIAutomation {
public:
    static IUIAutomation8* shared() {
        thread_local winrt::com_ptr<IUIAutomation8> instance;
        if (!instance) {
            THROW_IF_FAILED(CoCreateInstance(
                CLSID_CUIAutomation8, nullptr, CLSCTX_INPROC_SERVER,
                IID_PPV_ARGS(instance.put())));
        }
        return instance.get();
    }
};
```

UIA 在 MTA 下能用，但 event handler 必须由 STA 注册（见 6.3）。Query 类操作可以在主 MTA 线程跑。

### 6.2 文本抽取（focused window walk）

UIA 的 `ControlType` ↔ macOS AX `AXRole` 的对应：

| 含义 | macOS AXRole | Windows UIA ControlType |
|---|---|---|
| Window | `AXWindow` | `UIA_WindowControlTypeId` |
| Button | `AXButton` / `AXMenuButton` / `AXPopUpButton` | `UIA_ButtonControlTypeId` / `UIA_MenuItemControlTypeId` |
| TabGroup | `AXTabGroup` | `UIA_TabControlTypeId` |
| Static text | `AXStaticText` | `UIA_TextControlTypeId` |
| Edit | `AXTextField` / `AXTextArea` | `UIA_EditControlTypeId` |

**helper 内部用统一的归一化 RoleTag**（`Title` / `Button` / `Tab`），跨平台对外的 protocol 字段一致。

```cpp
class AccessibilityQuery {
public:
    static AxText query(DWORD pid, AxQueryOpts const& opts) {
        // 找 pid 对应 process 的 main window
        auto* uia = UIAutomation::shared();

        winrt::com_ptr<IUIAutomationCondition> pid_cond;
        VARIANT v; v.vt = VT_I4; v.lVal = static_cast<LONG>(pid);
        uia->CreatePropertyCondition(UIA_ProcessIdPropertyId, v, pid_cond.put());

        winrt::com_ptr<IUIAutomationCondition> window_cond;
        VARIANT vw; vw.vt = VT_I4; vw.lVal = UIA_WindowControlTypeId;
        uia->CreatePropertyCondition(UIA_ControlTypePropertyId, vw, window_cond.put());

        winrt::com_ptr<IUIAutomationCondition> and_cond;
        uia->CreateAndCondition(pid_cond.get(), window_cond.get(), and_cond.put());

        winrt::com_ptr<IUIAutomationElement> root;
        uia->GetRootElement(root.put());

        winrt::com_ptr<IUIAutomationElement> focused_window;
        // 优先 GetFocusedElement → 找 ancestor 是 window 的祖先
        uia->GetFocusedElement(focused_window.put());
        if (!focused_window) {
            // fallback：FindFirst by pid+window，挑 z-order 最高的
            root->FindFirst(TreeScope_Children, and_cond.get(), focused_window.put());
        }
        if (!focused_window) throw HelperError::not_found("no focused window for pid");

        std::string buffer;
        std::vector<RoleTagSpan> role_tags;
        auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(opts.deadline_ms);
        walk(focused_window.get(), 0, buffer, role_tags, deadline,
             opts.max_chars, opts.max_depth, opts.skip_predicate);

        return AxText{...};
    }

private:
    static void walk(IUIAutomationElement* el, int depth, std::string& buffer, ...) {
        if (depth > max_depth || ...) return;

        // 拿 role / name / value（对应 AX role / title / value）
        CONTROLTYPEID control_type;
        el->get_CurrentControlType(&control_type);
        BSTR name = nullptr;       el->get_CurrentName(&name);
        BSTR localized = nullptr;  el->get_CurrentLocalizedControlType(&localized);
        BSTR auto_id = nullptr;    el->get_CurrentAutomationId(&auto_id);

        SkipInfo info{control_type_to_role(control_type), nullopt, name ? bstr_to_utf8(name) : ""};
        if (skip_predicate(info)) { /* free strs */ return; }

        auto tag = role_tag_for(control_type);
        if (name && SysStringLen(name) > 0) {
            append_tagged(buffer, role_tags, tag, bstr_to_utf8(name));
        }

        // ValuePattern → value
        winrt::com_ptr<IUIAutomationValuePattern> vp;
        el->GetCurrentPatternAs(UIA_ValuePatternId, IID_PPV_ARGS(vp.put()));
        if (vp) {
            BSTR val = nullptr;
            vp->get_CurrentValue(&val);
            if (val && SysStringLen(val) > 0) {
                append_tagged(buffer, role_tags, std::nullopt, bstr_to_utf8(val));
                SysFreeString(val);
            }
        }

        // children（用 control view walker，跳过 noise）
        winrt::com_ptr<IUIAutomationTreeWalker> walker;
        UIAutomation::shared()->get_ControlViewWalker(walker.put());
        winrt::com_ptr<IUIAutomationElement> child;
        walker->GetFirstChildElement(el, child.put());
        while (child) {
            if (std::chrono::steady_clock::now() > deadline) break;
            if (buffer.size() >= max_chars) break;
            walk(child.get(), depth + 1, buffer, role_tags, deadline, ...);
            winrt::com_ptr<IUIAutomationElement> next;
            walker->GetNextSiblingElement(child.get(), next.put());
            child = next;
        }

        // free strs
    }
};
```

UIA 调用都可能 block（被 hung 的目标进程拖累），所以全程加 deadline。Win10+ 的 UIA 不支持 per-call timeout（unlike AX 的 `AXUIElementSetMessagingTimeout`），只能靠 deadline 提前 return + 用调用线程从主线程隔离避免拖死 stdin loop。

### 6.3 选区探测

```cpp
class SelectionProbe {
public:
    static std::optional<std::string> probe(DWORD pid) {
        auto* uia = UIAutomation::shared();
        winrt::com_ptr<IUIAutomationElement> focused;
        uia->GetFocusedElement(focused.put());
        if (!focused) return std::nullopt;

        DWORD focused_pid = 0;
        focused->get_CurrentProcessId(reinterpret_cast<int*>(&focused_pid));
        if (focused_pid != pid) return std::nullopt;  // 焦点不在请求的 pid 上

        winrt::com_ptr<IUIAutomationTextPattern> tp;
        focused->GetCurrentPatternAs(UIA_TextPatternId, IID_PPV_ARGS(tp.put()));
        if (!tp) return std::nullopt;  // 不支持 TextPattern

        winrt::com_ptr<IUIAutomationTextRangeArray> ranges;
        tp->GetSelection(ranges.put());
        if (!ranges) return std::nullopt;

        int count = 0; ranges->get_Length(&count);
        if (count == 0) return std::nullopt;

        std::string out;
        for (int i = 0; i < count; ++i) {
            winrt::com_ptr<IUIAutomationTextRange> range;
            ranges->GetElement(i, range.put());
            BSTR text = nullptr;
            range->GetText(/* maxLength */ -1, &text);
            if (text) {
                if (!out.empty()) out += "\n";
                out += bstr_to_utf8(text);
                SysFreeString(text);
            }
        }
        if (out.empty()) return std::nullopt;
        return out;
    }
};
```

UIA `TextPattern` 是 macOS `AXTextMarker` 的 Windows 等价物，正确处理跨节点选区。

### 6.4 UIA 事件订阅

UIA event handler 注册必须在 STA。开一根专用 STA worker：

```cpp
class AXThread {
public:
    void start() {
        thread_ = std::thread([this]{
            CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
            run_loop();
            CoUninitialize();
        });
    }

    void subscribe(DWORD pid, std::vector<std::string> notifications) {
        std::lock_guard lk(mu_);
        commands_.push(Command::Set{pid, std::move(notifications)});
        SetEvent(wake_.get());
    }

private:
    void run_loop() {
        auto* uia = UIAutomation::shared();
        std::optional<Subscription> current;

        while (running_) {
            // 1. drain commands
            std::queue<Command> batch;
            { std::lock_guard lk(mu_); std::swap(batch, commands_); }
            while (!batch.empty()) {
                std::visit([&](auto&& c){ apply_command(uia, current, c); }, batch.front());
                batch.pop();
            }

            // 2. pump: WaitForSingleObject on wake_ event with 100ms timeout
            //    UIA event callbacks 由内部线程触发，pump 不严格必要，但要让 STA 能接收 marshal
            MSG msg;
            while (PeekMessage(&msg, nullptr, 0, 0, PM_REMOVE)) DispatchMessage(&msg);
            WaitForSingleObject(wake_.get(), 100);
        }

        if (current) tear_down_subscription(uia, *current);
    }

    void apply_command(IUIAutomation8* uia,
                       std::optional<Subscription>& current,
                       Command::Set const& cmd) {
        if (current && current->pid == cmd.pid) return;
        if (current) tear_down_subscription(uia, *current);
        current = create_subscription(uia, cmd.pid, cmd.notifications);
    }

    Subscription create_subscription(IUIAutomation8* uia, DWORD pid, ...) {
        // FindFirst 找到 pid 的 root；AddFocusChangedEventHandler 等
        // event handler 是 IUIAutomationFocusChangedEventHandler 子类，其 HandleFocusChangedEvent
        // 会被 UIA 在某线程上 marshal 到 STA → emit 到 EventEmitter
    }
    // ...
};
```

**事件映射**：

| 协议 notification | UIA event |
|---|---|
| `focused_window` | `UIA_Window_WindowOpenedEventId` + `UIA_AutomationFocusChangedEventId` 复合 |
| `title_changed` | `UIA_AutomationPropertyChangedEventId` on `UIA_NamePropertyId` |

helper 内部维持"一次只订一个 pid"（同 macOS）。

## 7. 前台 app 监听（SetWinEventHook）

```cpp
class ForegroundMonitor {
public:
    void start() {
        thread_ = std::thread([this]{
            // SetWinEventHook 要求调用线程有 message pump
            hook_ = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
                nullptr, &ForegroundMonitor::win_event_proc,
                /* idProcess */ 0, /* idThread */ 0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS
            );

            MSG msg;
            while (running_ && GetMessageW(&msg, nullptr, 0, 0) > 0) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            UnhookWinEvent(hook_);
        });
    }

    void stop() {
        running_ = false;
        if (thread_.joinable()) {
            PostThreadMessageW(GetThreadId(thread_.native_handle()), WM_QUIT, 0, 0);
            thread_.join();
        }
    }

private:
    static void CALLBACK win_event_proc(HWINEVENTHOOK, DWORD event, HWND hwnd,
                                         LONG idObject, LONG idChild, DWORD, DWORD) {
        if (event != EVENT_SYSTEM_FOREGROUND) return;
        if (idObject != OBJID_WINDOW || idChild != CHILDID_SELF) return;

        DWORD pid = 0;
        GetWindowThreadProcessId(hwnd, &pid);

        // 拿 process info
        wchar_t path[MAX_PATH] = {};
        HANDLE proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        DWORD path_len = MAX_PATH;
        QueryFullProcessImageNameW(proc, 0, path, &path_len);
        CloseHandle(proc);

        // bundle_id 等价物：用 exe 路径 basename 或者用 AppUserModelID
        std::wstring bundle_id = derive_bundle_id(path);   // exe basename or AUMID

        // window title
        wchar_t title[256] = {};
        GetWindowTextW(hwnd, title, 256);

        // dedup（基于 bundle_id），同 macOS 行为
        if (last_bundle_ == bundle_id) return;
        last_bundle_ = bundle_id;

        EventEmitter::instance().send(ForegroundActivated{
            pid,
            utf16_to_utf8(bundle_id),
            utf16_to_utf8(extract_app_name(path)),
            utf16_to_utf8(title),
            std::chrono::system_clock::now()
        });
    }
};
```

`bundle_id` 在 Windows 上用什么：
- **首选 AppUserModelID**（AUMID）—— 通过 `IPropertyStore` 查询 `PKEY_AppUserModel_ID`。UWP / packaged apps 有；桌面 app 有时也设置
- **退化用 exe path basename**（`zoom.exe`、`teams.exe`、`chrome.exe`）—— 兼容性最好
- helper 同时返回 `bundle_id` (优先 AUMID 否则 exe basename) 和 `executable_path`，让 Rust 主侧白名单匹配可以两路都试

`foreground.current` 同步走 `GetForegroundWindow` + 同款解析。

## 8. Windows.Media.Ocr

```cpp
winrt::Windows::Foundation::IAsyncOperation<OcrResult>
WindowsOCR::run_async(std::filesystem::path image_path,
                      std::vector<std::string> languages) {
    // 选第一个可用语言（OcrEngine 一次只支持一种）
    winrt::Windows::Media::Ocr::OcrEngine engine{nullptr};
    for (auto const& lang_code : languages) {
        winrt::Windows::Globalization::Language lang{utf8_to_hstring(lang_code)};
        if (winrt::Windows::Media::Ocr::OcrEngine::IsLanguageSupported(lang)) {
            engine = winrt::Windows::Media::Ocr::OcrEngine::TryCreateFromLanguage(lang);
            break;
        }
    }
    if (!engine) {
        engine = winrt::Windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages();
    }
    if (!engine) throw HelperError::unsupported("no OCR engine for any requested language");

    auto file = co_await winrt::Windows::Storage::StorageFile::GetFileFromPathAsync(
        winrt::hstring{image_path.wstring()});
    auto stream = co_await file.OpenAsync(
        winrt::Windows::Storage::FileAccessMode::Read);
    auto decoder = co_await winrt::Windows::Graphics::Imaging::BitmapDecoder::CreateAsync(stream);
    auto bitmap = co_await decoder.GetSoftwareBitmapAsync();

    auto result = co_await engine.RecognizeAsync(bitmap);

    // result.Lines() → 用相同的 stitch 算法重排
    std::vector<TextObservation> obs;
    for (auto const& line : result.Lines()) {
        // 取 line 里所有 word 的 bounding rect 并集
        auto rect = union_of(line.Words());
        obs.push_back({
            winrt::to_string(line.Text()),
            (rect.X + rect.Width / 2.0) / image_width,
            1.0 - (rect.Y + rect.Height / 2.0) / image_height,  // 翻转 y 轴匹配 macOS Vision 坐标系
            rect.Height / image_height
        });
    }
    co_return OcrResult{stitch(obs), elapsed_ms};
}
```

**语言包**：Windows.Media.Ocr 的中文（简/繁）需要用户在 Settings > Time & Language > Language 添加对应"语言包"中的 OCR 组件。helper 的 `capability` 里上报 `ocr_languages: ["en-US", "zh-Hans"]` 之类，让 Rust 主侧根据可用语言决定走 helper OCR 还是退化到云。

## 9. 错误转换

```cpp
struct HelperError : std::exception {
    std::string code;       // PROTOCOL code
    std::string message;
    nlohmann::json detail;
};

// HRESULT → ResponseError
ResponseError hresult_to_response_error(HRESULT hr, std::string_view ctx) {
    return ResponseError{
        "OS_ERROR",
        std::string{ctx} + ": " + hresult_to_string(hr),
        { {"hresult", static_cast<uint32_t>(hr)},
          {"facility", HRESULT_FACILITY(hr)},
          {"code", HRESULT_CODE(hr)} }
    };
}
```

`hresult_to_string` 用 `FormatMessageW(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_ALLOCATE_BUFFER, ...)`。

## 10. 权限处理

| 权限 | 检测 | 引导 |
|---|---|---|
| 麦克风 (Settings > Privacy > Microphone) | `Windows.Devices.Enumeration.DeviceAccessInformation::CreateFromDeviceClass(DeviceClass::AudioCapture).CurrentStatus` | 调用 `Launcher::LaunchUriAsync(L"ms-settings:privacy-microphone")` 跳设置 |
| 屏幕捕获 | n/a，无需权限 | n/a |
| UIA | n/a，无需权限（除非主进程是 UAC 提权 → helper 也得提权才能读 elevated 进程） | onboarding 不引导，遇到 elevated app 时 emit warning |

```cpp
PermissionStatus query_microphone() {
    auto info = winrt::Windows::Devices::Enumeration::DeviceAccessInformation::
        CreateFromDeviceClass(winrt::Windows::Devices::Enumeration::DeviceClass::AudioCapture);
    switch (info.CurrentStatus()) {
        case DeviceAccessStatus::Allowed:        return PermissionStatus::Granted;
        case DeviceAccessStatus::DeniedBySystem:
        case DeviceAccessStatus::DeniedByUser:   return PermissionStatus::Denied;
        case DeviceAccessStatus::Unspecified:    return PermissionStatus::Undetermined;
    }
}
```

`permission.request{kind:"microphone"}` 通过 `Launcher::LaunchUriAsync` 打开 Settings 应用对应面板（无 silent prompt）。

## 11. Manifest

`CorivoCaptureHelper.manifest`：

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="com.corivo.capture-helper" version="0.1.0.0"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/> <!-- Win10 -->
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/> <!-- Win11 -->
    </application>
  </compatibility>
</assembly>
```

**`asInvoker`**：永不自提权（提权 helper 反而会破坏 sidecar 父子关系）。

**Per-monitor DPI v2**：让 WGC / display enumeration 拿到的尺寸是物理像素，避免高 DPI 下截图模糊。

## 12. 构建 & 打包

`CMakeLists.txt`：

```cmake
cmake_minimum_required(VERSION 3.26)
project(CorivoCaptureHelper LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# vcpkg
find_package(nlohmann_json CONFIG REQUIRED)

# cppwinrt
find_package(cppwinrt CONFIG REQUIRED)

add_executable(CorivoCaptureHelper
    src/main.cpp
    src/ipc/codec.cpp
    src/ipc/router.cpp
    src/ipc/event_emitter.cpp
    # ... 全部 .cpp
    CorivoCaptureHelper.manifest
    resource.rc
)

target_link_libraries(CorivoCaptureHelper PRIVATE
    nlohmann_json::nlohmann_json
    windowsapp.lib
    mmdevapi.lib audioses.lib
    mfplat.lib mfreadwrite.lib mfuuid.lib mf.lib
    d3d11.lib dxgi.lib
    uiautomationcore.lib
    propsys.lib
    avrt.lib                # AvSetMmThreadCharacteristics
)

target_compile_options(CorivoCaptureHelper PRIVATE
    /W4 /WX /permissive-
    /await:strict           # C++/WinRT coroutines
    /utf-8
)

target_compile_definitions(CorivoCaptureHelper PRIVATE
    NOMINMAX
    WIN32_LEAN_AND_MEAN
    _WIN32_WINNT=0x0A00     # Win10
)

# Subsystem console（保留 stdio）
set_target_properties(CorivoCaptureHelper PROPERTIES
    LINK_FLAGS "/SUBSYSTEM:CONSOLE"
)
```

`vcpkg.json`：

```json
{
  "name": "corivo-capture-helper",
  "version": "0.1.0",
  "dependencies": ["nlohmann-json", "wil"]
}
```

构建脚本 `helpers/windows/build.ps1`：

```powershell
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# vcpkg bootstrap if needed
if (-not (Test-Path "vcpkg/bootstrap-vcpkg.bat")) {
    git clone https://github.com/microsoft/vcpkg.git
    .\vcpkg\bootstrap-vcpkg.bat
}

cmake -B build -S . -G "Visual Studio 17 2022" -A x64 `
    -DCMAKE_TOOLCHAIN_FILE="vcpkg/scripts/buildsystems/vcpkg.cmake"
cmake --build build --config Release

# Sign
$cert = "..\..\..\..\codesign\corivo-ev-cert.pfx"  # EV cert path
signtool sign /f $cert /tr http://timestamp.digicert.com /td sha256 /fd sha256 `
    "build\Release\CorivoCaptureHelper.exe"

# 拷到 Tauri sidecar 目录，加 target triple 后缀
$dest = "..\..\src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item "build\Release\CorivoCaptureHelper.exe" `
    "$dest\corivo-capture-helper-x86_64-pc-windows-msvc.exe"
```

## 13. 测试

### 13.1 单元（GoogleTest）

每模块 ≥ 1 文件：
- `protocol_tests.cpp`：v1 schema round-trip
- `segment_writer_tests.cpp`：fake PCM 流 + virtual filesystem，验证 segment 切分
- `accessibility_walk_tests.cpp`：mock `IUIAutomationElement` 实现，验证 walk + role tag mapping
- `hresult_mapping_tests.cpp`

### 13.2 端到端（CTest，需要 Windows runner）

- 录 30s session：assert ≥ 3 segments、`MFCreateSourceReaderFromURL` 读出 2 streams
- 截图 primary monitor：assert PNG 文件 + 物理像素尺寸
- UIA query 当前 Explorer.exe pid：assert 有 `[TITLE]` tag
- 1 小时 soak（nightly）：内存 / segment 数 / 崩溃恢复

### 13.3 协议合规

`--echo-mode` 同 macOS：Rust 端的 `tests/capture_helper_protocol.rs` 跑真实 helper exe 的 echo-mode，全协议 round-trip。同一个测试在 macOS 和 Windows runner 上分别跑各自的 helper binary，验证 Rust 端实现完全 platform-agnostic。

## 14. 性能预算

| 操作 | 预算 | 监控 |
|---|---|---|
| `screen.capture` 1 帧 4K | < 250 ms | per-call elapsed_ms |
| `ax.query` 典型 8 KB 输出 | < 700 ms（UIA 比 AX 慢） | 同上 |
| `ocr.run` 4K 截图（Windows OCR） | < 2000 ms | 同上 |
| 录音稳态 CPU（sys+mic+AAC） | < 6% on i7 | helper 进程 perfmon |
| 录音稳态内存 | < 100 MB | working set |
| Segment 切换抖动 | < 20 ms 间隙 | mediainfo segment N vs N+1 |

UIA 比 AX 慢是已知的；如果 ax_query 实测 > 1.5s，要回头加 cache（UIA 的 `IUIAutomationCacheRequest` 一次拉所有需要的 property，避免逐节点 RPC）。

## 15. 已知风险 & 留白

| 风险 | 处理 |
|---|---|
| Windows N/KN editions 缺 Media Feature Pack → 无 AAC 编码器 | capability 探测 + onboarding 引导（§ 4.5） |
| UIA 在 Win10 老版本对 Chromium based browser 性能差 | Phase 2 测试 Chrome / Edge / Brave 实测；不行 Phase 3 加 IUIAutomationCacheRequest |
| WGC 在 Win10 早期版本 (1903-1909) 不支持 cursor capture toggle | runtime 用 `ApiInformation::IsPropertyPresent` 软探测，缺失时光标也录进去 |
| 用户主进程是 elevated 的 (admin) → helper as_invoker 读不了 elevated app 的 UIA tree | helper emit `warning` event；onboarding 引导用户"不要以 admin 跑 Corivo" |
| WASAPI loopback 在某些 USB DAC 上 GetMixFormat 给出非 PCM float | start 时检查 + 调 IAudioClient::IsFormatSupported 协商；失败上报 OS_ERROR |
| EV signing cert 私钥保管 | 同主 app 走同一个 HSM / CI secret；helper 不引入新证书 |

---

## 16. 与 macOS spec 的差异速查

| 维度 | macOS | Windows |
|---|---|---|
| 系统音频 | SCStream（必须挂 1x1 video filter） | WASAPI loopback（纯音频原生） |
| 麦克风 | AVCaptureSession | WASAPI capture |
| 截图 | SCK SCScreenshotManager | WGC + D3D + WIC |
| Accessibility 文本 | AXUIElement | IUIAutomation |
| Accessibility 选区 | AXTextMarker（参数化属性） | IUIAutomationTextPattern |
| Accessibility 事件 | AXObserver + CFRunLoop | UIA event handler + STA |
| 前台监听 | NSWorkspace observer | SetWinEventHook |
| OCR | Vision VNRecognizeTextRequest | Windows.Media.Ocr |
| 编码器 | AVAssetWriter | MF Sink Writer |
| 多轨 | 多 AVAssetWriterInput | 多 AddStream |
| 容器 | .m4a (ISO base media) | .m4a (同一容器) |
| 权限 - mic | NSMicrophoneUsageDescription | Settings privacy + DeviceAccessInformation |
| 权限 - 截屏 | Screen Recording | n/a |
| 权限 - a11y | Accessibility (per bundle id) | n/a |
| 签名 | Developer ID Application + notarize | EV code signing |
| Bundle 形态 | .app 嵌入主 app | 单 .exe |
| 进程线程模型 | RunLoop.main + dedicated CFRunLoop thread | MTA + dedicated STA worker |

**对外 IPC 协议完全一致** —— 这是 helper 架构的全部价值所在。Rust 主侧不应该出现任何 `#[cfg(target_os = ...)]`。
