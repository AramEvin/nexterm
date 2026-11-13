#!/usr/bin/env bash
# Build the Windows NSIS installer (NexTerm_<ver>_x64-setup.exe) from a Linux host.
#
# Tauri calls this "cross-compilation (experimental)". Only NSIS works from Linux;
# MSI needs WiX, which runs on Windows only. The official release path stays the
# Windows CI job in .github/workflows/ci.yml.
#
# Prereqs (Debian/Ubuntu names; on Oracle Linux 9: mingw64-gcc, nsis from EPEL):
#   sudo apt-get install nsis mingw-w64
#   rustup target add x86_64-pc-windows-gnu
#   ~/.cargo/config.toml:
#     [target.x86_64-pc-windows-gnu]
#     linker = "x86_64-w64-mingw32-gcc"
#
# If `rustup target add` is impossible (offline/firewalled), set BUILD_STD=1 and put
# the matching rust-lang/rust `library/` tree under
# $(rustc --print sysroot)/lib/rustlib/src/rust/ - std is then compiled from source.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=x86_64-pc-windows-gnu
EXTRA=()
if [[ "${BUILD_STD:-0}" == "1" ]]; then
  export RUSTC_BOOTSTRAP=1
  EXTRA=(-Zbuild-std=std,panic_abort)
fi

npm ci
npm run build
(cd src-tauri && cargo build --release --locked --features tauri/custom-protocol --target "$TARGET" "${EXTRA[@]}")
npx tauri bundle --target "$TARGET" --bundles nsis

echo
echo "Installer: $(ls src-tauri/target/$TARGET/release/bundle/nsis/*-setup.exe)"
