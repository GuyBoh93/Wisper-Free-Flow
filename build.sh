#!/usr/bin/env bash
# Whisper FreeFlow build script (macOS / Linux).
# Usage:
#   ./build.sh             - debug build, runs the app
#   ./build.sh release     - optimized release build (no run)
#   ./build.sh installer   - release build + produce .dmg (macOS) / .AppImage (Linux)
#   ./build.sh clean       - clean target/ directory

set -euo pipefail

# Make sure cargo is on PATH (rustup installs to ~/.cargo/bin).
if [ -z "${CARGO_HOME:-}" ]; then
    export CARGO_HOME="$HOME/.cargo"
fi
export PATH="$CARGO_HOME/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
    echo "[build.sh] cargo not found. Install Rust from https://rustup.rs/ first."
    exit 1
fi

# whisper-rs builds whisper.cpp from source via cmake.
if ! command -v cmake >/dev/null 2>&1; then
    echo "[build.sh] WARN: cmake not on PATH. whisper-rs may fail to build."
    echo "  macOS: brew install cmake"
    echo "  Linux: apt install cmake (or your distro's equivalent)"
fi

case "${1:-}" in
    clean)
        cargo clean
        ;;
    release)
        cargo build --release
        ;;
    installer)
        if ! command -v cargo-packager >/dev/null 2>&1; then
            echo "[build.sh] Installing cargo-packager..."
            cargo install cargo-packager --locked
        fi
        case "$(uname -s)" in
            Darwin) cargo packager --release --verbose --formats dmg ;;
            Linux)  cargo packager --release --verbose --formats appimage ;;
            *)      cargo packager --release --verbose ;;
        esac
        ;;
    "")
        cargo run
        ;;
    *)
        echo "[build.sh] unknown command: $1"
        echo "Usage: $0 [clean|release|installer]"
        exit 1
        ;;
esac
