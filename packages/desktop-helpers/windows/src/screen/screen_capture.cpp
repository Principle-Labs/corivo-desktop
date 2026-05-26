// screen_capture.cpp
//
// WGC pipeline:
//
//   1. Find a GraphicsCaptureItem for the target monitor via the
//      IGraphicsCaptureItemInterop COM glue (no UI prompt path is exposed
//      from the activation factory itself for monitors).
//   2. Create a D3D11 device + Direct3D11Device (cppwinrt's interop helper).
//   3. Create a Direct3D11CaptureFramePool (FreeThreaded variant — we
//      don't have a UI dispatcher).
//   4. Hook FrameArrived, StartCapture, wait for the first frame.
//   5. Copy the GPU texture into a CPU-readable staging texture, then
//      encode via WIC into the requested format on disk.
//
// Phase 1 keeps it strictly one-shot: capture closes immediately after
// the first frame. Multi-frame / streaming lives in Phase 5 (recording).

#include "screen_capture.hpp"
#include "display_enumerator.hpp"
#include "../util/logger.hpp"

#include <stdexcept>
#include <string>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

#include <wrl/client.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wincodec.h>
#include <ShlObj.h>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
// ApiInformation::IsPropertyPresent lives in this projection — without
// it, the cursor-capture-disable guard below references an incomplete
// `winrt::Windows::Foundation::Metadata` namespace.
#include <winrt/Windows.Foundation.Metadata.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>

#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>

namespace corivo::screen {

namespace {

using Microsoft::WRL::ComPtr;

void throw_if_failed(HRESULT hr, const char* where) {
    if (FAILED(hr)) {
        char buf[160];
        std::snprintf(buf, sizeof(buf), "%s failed: HRESULT 0x%08lX", where,
                      static_cast<unsigned long>(hr));
        throw std::runtime_error(buf);
    }
}

std::wstring utf8_to_utf16(const std::string& s) {
    if (s.empty()) return {};
    int len = MultiByteToWideChar(CP_UTF8, 0, s.data(),
                                   static_cast<int>(s.size()),
                                   nullptr, 0);
    std::wstring out(len, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()),
                        out.data(), len);
    return out;
}

std::string resolve_output_path(const std::optional<std::string>& given,
                                ImageFormat format) {
    if (given && !given->empty()) return *given;
    wchar_t temp[MAX_PATH];
    DWORD n = GetTempPathW(MAX_PATH, temp);
    if (n == 0 || n >= MAX_PATH) {
        throw std::runtime_error("GetTempPathW failed");
    }
    GUID g;
    CoCreateGuid(&g);
    wchar_t guid_buf[64];
    StringFromGUID2(g, guid_buf, 64);
    std::wstring path = std::wstring(temp) + L"corivo-helper-" + guid_buf +
                        (format == ImageFormat::Jpeg ? L".jpg" : L".png");
    int u8 = WideCharToMultiByte(CP_UTF8, 0, path.c_str(), -1, nullptr, 0,
                                  nullptr, nullptr);
    std::string out;
    out.resize(u8 - 1);
    WideCharToMultiByte(CP_UTF8, 0, path.c_str(), -1, out.data(), u8,
                        nullptr, nullptr);
    return out;
}

ComPtr<ID3D11Device> create_d3d_device() {
    UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    D3D_FEATURE_LEVEL level;
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> ctx;
    HRESULT hr = D3D11CreateDevice(
        nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags,
        nullptr, 0, D3D11_SDK_VERSION,
        device.GetAddressOf(), &level, ctx.GetAddressOf());
    if (FAILED(hr)) {
        // Fall back to WARP for headless / VM environments.
        hr = D3D11CreateDevice(
            nullptr, D3D_DRIVER_TYPE_WARP, nullptr, flags,
            nullptr, 0, D3D11_SDK_VERSION,
            device.GetAddressOf(), &level, ctx.GetAddressOf());
    }
    throw_if_failed(hr, "D3D11CreateDevice");
    return device;
}

winrt::Windows::Graphics::DirectX::Direct3D11::IDirect3DDevice
to_winrt_direct3d_device(ID3D11Device* device) {
    ComPtr<IDXGIDevice> dxgi;
    throw_if_failed(device->QueryInterface(IID_PPV_ARGS(dxgi.GetAddressOf())),
                    "ID3D11Device::QI(IDXGIDevice)");
    winrt::com_ptr<::IInspectable> inspectable;
    throw_if_failed(
        CreateDirect3D11DeviceFromDXGIDevice(dxgi.Get(),
                                              reinterpret_cast<::IInspectable**>(
                                                  winrt::put_abi(inspectable))),
        "CreateDirect3D11DeviceFromDXGIDevice");
    return inspectable.as<
        winrt::Windows::Graphics::DirectX::Direct3D11::IDirect3DDevice>();
}

ComPtr<ID3D11Texture2D> texture_from_winrt_surface(
    winrt::Windows::Graphics::DirectX::Direct3D11::IDirect3DSurface const& surface) {
    auto access = surface.as<
        ::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
    ComPtr<ID3D11Texture2D> tex;
    throw_if_failed(access->GetInterface(IID_PPV_ARGS(tex.GetAddressOf())),
                    "IDirect3DDxgiInterfaceAccess::GetInterface");
    return tex;
}

void encode_to_file(const std::string& utf8_path,
                    ImageFormat format,
                    int quality,
                    const std::vector<BYTE>& bgra,
                    UINT width,
                    UINT height,
                    UINT stride) {
    ComPtr<IWICImagingFactory> wic;
    throw_if_failed(
        CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                         IID_PPV_ARGS(wic.GetAddressOf())),
        "CoCreateInstance(IWICImagingFactory)");

    ComPtr<IWICStream> stream;
    throw_if_failed(wic->CreateStream(stream.GetAddressOf()),
                    "WIC CreateStream");

    std::wstring wpath = utf8_to_utf16(utf8_path);
    throw_if_failed(stream->InitializeFromFilename(wpath.c_str(), GENERIC_WRITE),
                    "WIC InitializeFromFilename");

    ComPtr<IWICBitmapEncoder> encoder;
    GUID container = (format == ImageFormat::Jpeg)
                         ? GUID_ContainerFormatJpeg
                         : GUID_ContainerFormatPng;
    throw_if_failed(
        wic->CreateEncoder(container, nullptr, encoder.GetAddressOf()),
        "WIC CreateEncoder");
    throw_if_failed(encoder->Initialize(stream.Get(), WICBitmapEncoderNoCache),
                    "WIC encoder Initialize");

    ComPtr<IWICBitmapFrameEncode> frame;
    ComPtr<IPropertyBag2> bag;
    throw_if_failed(
        encoder->CreateNewFrame(frame.GetAddressOf(), bag.GetAddressOf()),
        "WIC CreateNewFrame");

    if (format == ImageFormat::Jpeg && bag) {
        PROPBAG2 opt{};
        opt.pstrName = const_cast<LPOLESTR>(L"ImageQuality");
        VARIANT v;
        VariantInit(&v);
        v.vt = VT_R4;
        v.fltVal = std::max(0, std::min(100, quality)) / 100.0f;
        bag->Write(1, &opt, &v);
    }

    throw_if_failed(frame->Initialize(bag.Get()), "WIC frame Initialize");
    throw_if_failed(frame->SetSize(width, height), "WIC SetSize");

    WICPixelFormatGUID pixel_format = GUID_WICPixelFormat32bppBGRA;
    throw_if_failed(frame->SetPixelFormat(&pixel_format),
                    "WIC SetPixelFormat");

    throw_if_failed(
        frame->WritePixels(height, stride,
                           static_cast<UINT>(bgra.size()),
                           const_cast<BYTE*>(bgra.data())),
        "WIC WritePixels");
    throw_if_failed(frame->Commit(), "WIC frame Commit");
    throw_if_failed(encoder->Commit(), "WIC encoder Commit");
}

} // namespace

CaptureResponse capture(const CaptureRequest& request) {
    // 1. Resolve target monitor.
    DisplayHandle target =
        resolve(request.display_id.value_or(std::string{}));
    HMONITOR hmon = static_cast<HMONITOR>(target.hmonitor);

    // 2. Create D3D + WinRT device wrapper.
    ComPtr<ID3D11Device> d3d = create_d3d_device();
    auto winrt_device = to_winrt_direct3d_device(d3d.Get());

    // 3. GraphicsCaptureItem for the monitor (interop factory path).
    auto interop = winrt::get_activation_factory<
        winrt::Windows::Graphics::Capture::GraphicsCaptureItem,
        ::IGraphicsCaptureItemInterop>();
    winrt::Windows::Graphics::Capture::GraphicsCaptureItem item{nullptr};
    HRESULT hr = interop->CreateForMonitor(
        hmon,
        winrt::guid_of<winrt::Windows::Graphics::Capture::GraphicsCaptureItem>(),
        winrt::put_abi(item));
    throw_if_failed(hr, "IGraphicsCaptureItemInterop::CreateForMonitor");

    auto size = item.Size();

    // 4. Frame pool + session. FreeThreaded variant: no UI dispatcher needed.
    auto pool = winrt::Windows::Graphics::Capture::Direct3D11CaptureFramePool::
        CreateFreeThreaded(
            winrt_device,
            winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size);
    auto session = pool.CreateCaptureSession(item);

    if (winrt::Windows::Foundation::Metadata::ApiInformation::IsPropertyPresent(
            L"Windows.Graphics.Capture.GraphicsCaptureSession",
            L"IsCursorCaptureEnabled")) {
        session.IsCursorCaptureEnabled(false);
    }

    // Suppress the yellow capture-indicator border WGC draws around the
    // captured monitor. Unpackaged Win32 apps can opt out directly; no
    // consent prompt. Property exists on Windows 11 22621+.
    if (winrt::Windows::Foundation::Metadata::ApiInformation::IsPropertyPresent(
            L"Windows.Graphics.Capture.GraphicsCaptureSession",
            L"IsBorderRequired")) {
        session.IsBorderRequired(false);
    }

    HANDLE arrived = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!arrived) {
        throw std::runtime_error("CreateEvent failed: " +
                                 std::to_string(GetLastError()));
    }

    winrt::Windows::Graphics::Capture::Direct3D11CaptureFrame captured{nullptr};
    auto token = pool.FrameArrived(
        [&](auto const& sender, auto const&) {
            auto frame = sender.TryGetNextFrame();
            if (frame) {
                captured = frame;
                SetEvent(arrived);
            }
        });

    session.StartCapture();

    DWORD wait = WaitForSingleObject(arrived, 2000);
    pool.FrameArrived(token);  // detach
    session.Close();
    pool.Close();
    CloseHandle(arrived);

    if (wait != WAIT_OBJECT_0 || !captured) {
        throw std::runtime_error("WGC frame did not arrive within 2s");
    }

    // 5. Copy to staging + encode.
    ComPtr<ID3D11Texture2D> gpu_tex = texture_from_winrt_surface(captured.Surface());
    D3D11_TEXTURE2D_DESC desc{};
    gpu_tex->GetDesc(&desc);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    desc.MiscFlags = 0;

    ComPtr<ID3D11Texture2D> staging;
    throw_if_failed(d3d->CreateTexture2D(&desc, nullptr, staging.GetAddressOf()),
                    "CreateTexture2D(staging)");

    ComPtr<ID3D11DeviceContext> ctx;
    d3d->GetImmediateContext(ctx.GetAddressOf());
    ctx->CopyResource(staging.Get(), gpu_tex.Get());

    D3D11_MAPPED_SUBRESOURCE mapped{};
    throw_if_failed(ctx->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapped),
                    "Map(staging)");

    UINT row_bytes = desc.Width * 4;
    std::vector<BYTE> bgra(static_cast<size_t>(row_bytes) * desc.Height);
    BYTE* src = static_cast<BYTE*>(mapped.pData);
    for (UINT y = 0; y < desc.Height; ++y) {
        std::memcpy(bgra.data() + static_cast<size_t>(y) * row_bytes,
                    src + static_cast<size_t>(y) * mapped.RowPitch,
                    row_bytes);
    }
    ctx->Unmap(staging.Get(), 0);

    std::string out_path = resolve_output_path(request.output_path, request.format);
    encode_to_file(out_path, request.format, request.quality.value_or(80),
                   bgra, desc.Width, desc.Height, row_bytes);

    char id_buf[32];
    std::snprintf(id_buf, sizeof(id_buf), "%llu",
                  static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(hmon)));

    CaptureResponse resp;
    resp.path = out_path;
    resp.width = static_cast<int>(desc.Width);
    resp.height = static_cast<int>(desc.Height);
    resp.captured_at = std::chrono::system_clock::now();
    resp.display_id = id_buf;
    return resp;
}

} // namespace corivo::screen
