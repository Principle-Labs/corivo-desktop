# Build the Windows Capture Helper into apps/desktop/src-tauri/binaries/
# under the names Tauri sidecar resolution expects.
#
# Usage:
#   ./build.ps1            # debug build
#   ./build.ps1 Release    # release build
#
# Requires: CMake 3.20+, Visual Studio 2022 (Build Tools or full IDE),
# Git (for the FetchContent of nlohmann/json).
#
# Phase 0 scope: produce a runnable .exe. No code signing, no .manifest
# (UAC manifest is only meaningful when permissions land in later phases).

$ErrorActionPreference = "Stop"
Set-Location -Path $PSScriptRoot

$Config = "Debug"
if ($args.Count -ge 1) {
    $Config = $args[0]
    if ($Config -ne "Debug" -and $Config -ne "Release") {
        Write-Error "build.ps1: unknown config '$Config' (expected Debug|Release)"
        exit 1
    }
}

Write-Host "[build.ps1] cmake configure (Visual Studio 17 2022, x64)"
cmake -B build -S . -G "Visual Studio 17 2022" -A x64

Write-Host "[build.ps1] cmake --build build --config $Config"
cmake --build build --config $Config

$exe = Join-Path $PSScriptRoot "build\$Config\CorivoCaptureHelper.exe"
if (-not (Test-Path $exe)) {
    Write-Error "[build.ps1] expected built binary at $exe but it's missing"
    exit 1
}

$dest = Resolve-Path (Join-Path $PSScriptRoot "..\..\..\apps\desktop\src-tauri\binaries")
New-Item -ItemType Directory -Force -Path $dest | Out-Null

# Tauri sidecar naming convention: <prefix>-<target-triple>.exe.
# x86_64-pc-windows-msvc is the canonical Windows triple.
$targets = @("x86_64-pc-windows-msvc")
foreach ($triple in $targets) {
    $out = Join-Path $dest "corivo-capture-helper-$triple.exe"
    Copy-Item -Path $exe -Destination $out -Force
    Write-Host "[build.ps1] -> $out"
}

Write-Host "[build.ps1] done"
