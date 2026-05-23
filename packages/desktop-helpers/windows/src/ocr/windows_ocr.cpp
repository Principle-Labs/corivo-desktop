// windows_ocr.cpp

#include "windows_ocr.hpp"

#include <chrono>
#include <stdexcept>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
// MultiByteToWideChar + CP_UTF8 are declared in <stringapiset.h>, pulled
// in transitively via <Windows.h>. Without this the UTF-8 ↔ UTF-16
// helpers below fail to compile.
#include <Windows.h>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Globalization.h>
#include <winrt/Windows.Media.Ocr.h>
#include <winrt/Windows.Graphics.Imaging.h>
#include <winrt/Windows.Storage.h>
#include <winrt/Windows.Storage.Streams.h>

namespace corivo::ocr {

namespace {

std::wstring utf8_to_utf16(const std::string& s) {
    if (s.empty()) return {};
    int len = MultiByteToWideChar(CP_UTF8, 0, s.data(),
                                   static_cast<int>(s.size()),
                                   nullptr, 0);
    std::wstring out(len, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(),
                        static_cast<int>(s.size()),
                        out.data(), len);
    return out;
}

std::string winrt_to_utf8(winrt::hstring const& h) {
    int wlen = static_cast<int>(h.size());
    if (wlen == 0) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, h.c_str(), wlen,
                                   nullptr, 0, nullptr, nullptr);
    std::string out(len, '\0');
    WideCharToMultiByte(CP_UTF8, 0, h.c_str(), wlen,
                        out.data(), len, nullptr, nullptr);
    return out;
}

} // namespace

Response run(const Request& req) {
    using namespace winrt;
    using namespace winrt::Windows::Foundation;
    using namespace winrt::Windows::Globalization;
    using namespace winrt::Windows::Graphics::Imaging;
    using namespace winrt::Windows::Media::Ocr;
    using namespace winrt::Windows::Storage;

    auto started = std::chrono::steady_clock::now();

    OcrEngine engine{nullptr};
    for (auto const& lang_code : req.languages) {
        try {
            Language lang{utf8_to_utf16(lang_code)};
            if (OcrEngine::IsLanguageSupported(lang)) {
                engine = OcrEngine::TryCreateFromLanguage(lang);
                if (engine) break;
            }
        } catch (...) { /* skip; try next language */ }
    }
    if (!engine) {
        engine = OcrEngine::TryCreateFromUserProfileLanguages();
    }
    if (!engine) {
        throw std::runtime_error(
            "no OCR engine available; install a Windows language pack with OCR");
    }

    auto file = StorageFile::GetFileFromPathAsync(
                    hstring{utf8_to_utf16(req.image_path)})
                    .get();
    auto stream = file.OpenAsync(FileAccessMode::Read).get();
    auto decoder = BitmapDecoder::CreateAsync(stream).get();
    auto bitmap = decoder.GetSoftwareBitmapAsync().get();

    auto result = engine.RecognizeAsync(bitmap).get();

    std::string out;
    out.reserve(2048);
    for (auto const& line : result.Lines()) {
        if (!out.empty()) out.push_back('\n');
        out.append(winrt_to_utf8(line.Text()));
    }

    Response resp;
    resp.text = std::move(out);
    resp.elapsed_ms = static_cast<unsigned long long>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now() - started).count());
    return resp;
}

} // namespace corivo::ocr
