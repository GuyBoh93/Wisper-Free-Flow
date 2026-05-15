@echo off
setlocal

rem Whisper FreeFlow build script
rem Usage:
rem   build.bat           - debug build, runs the app
rem   build.bat release   - optimized release build (no run)
rem   build.bat installer - release build + produce .msi/.exe installers
rem   build.bat clean     - clean target/ directory

rem ---- Configure toolchain paths ----
if not defined CARGO_HOME set "CARGO_HOME=%USERPROFILE%\.cargo"
set "PATH=%CARGO_HOME%\bin;%PATH%"

rem CMake (needed by whisper-rs build script)
if exist "C:\Program Files\CMake\bin\cmake.exe" set "PATH=C:\Program Files\CMake\bin;%PATH%"

rem LLVM (libclang for bindgen, used by whisper-rs)
if exist "C:\Program Files\LLVM\bin\libclang.dll" (
    set "PATH=C:\Program Files\LLVM\bin;%PATH%"
    if not defined LIBCLANG_PATH set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
)

where cargo >nul 2>nul
if errorlevel 1 (
    echo [build.bat] cargo not found. Install Rust from https://rustup.rs/ first.
    exit /b 1
)

where cmake >nul 2>nul
if errorlevel 1 (
    echo [build.bat] WARN: cmake not on PATH. whisper-rs may fail to build.
)

if not defined LIBCLANG_PATH (
    echo [build.bat] WARN: LIBCLANG_PATH not set. bindgen may fail. Install LLVM.
)

if /i "%1"=="clean" (
    cargo clean
    exit /b %ERRORLEVEL%
)

if /i "%1"=="release" (
    cargo build --release
    exit /b %ERRORLEVEL%
)

if /i "%1"=="installer" (
    where cargo-packager >nul 2>nul
    if errorlevel 1 (
        echo [build.bat] Installing cargo-packager...
        cargo install cargo-packager --locked
        if errorlevel 1 exit /b %ERRORLEVEL%
    )
    cargo packager --release --verbose
    exit /b %ERRORLEVEL%
)

rem Default: debug build + run
cargo run
exit /b %ERRORLEVEL%
