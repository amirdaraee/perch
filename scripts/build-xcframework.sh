#!/usr/bin/env bash
# Build perch-ffi as a static library for macOS (arm64, plus x86_64 when that
# Rust target is available), generate the Swift bindings, and package an
# XCFramework the SwiftPM app links against.
#
# Bindings are generated FROM THE COMPILED LIBRARY (not from source) because
# that is what UniFFI's `uniffi-bindgen-swift` CLI does in 0.32: it reads the
# UniFFI metadata that `uniffi::setup_scaffolding!()` embeds into the .a at
# build time. Generating from the library rather than from a separate UDL/proc
# macro pass guarantees the emitted Swift always matches the exact bytes
# Task 7 links against - there is no separate "did I forget to regenerate"
# step that can drift.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=release
ALL_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
OUT=apps/macos/Perch
FW="$OUT/Frameworks/PerchCore.xcframework"
GEN="$OUT/Sources/PerchFFI"
BUILD=build/xcframework
LIB=libperch_ffi.a

# --- Resolve which Rust targets we can actually build for. -----------------
#
# The native target (aarch64-apple-darwin on Apple Silicon) is assumed to
# already be installed - if it isn't, or the build for it fails, that's a
# real error and the script should stop, not silently produce an empty
# framework. The x86_64 slice is best-effort: `rustup target add` needs a
# network fetch the first time, and a machine without network access for it
# should still come away with a usable (arm64-only) artefact rather than a
# script that dies halfway through with a confusing lipo/cargo error. CI
# machines with network access get the full universal build for free because
# this loop is the only place target selection happens - nothing below it is
# arm64-specific.
TARGETS=()
for t in "${ALL_TARGETS[@]}"; do
  if rustup target list --installed | grep -qx "$t"; then
    TARGETS+=("$t")
  elif rustup target add "$t" >/dev/null 2>&1; then
    echo "fetched rust target $t"
    TARGETS+=("$t")
  else
    echo "warning: rust target $t is not installed and could not be fetched (no network?) - building without that slice" >&2
  fi
done

if [[ ${#TARGETS[@]} -eq 0 ]]; then
  echo "error: no usable rust target found (expected at least aarch64-apple-darwin)" >&2
  exit 1
fi

for t in "${TARGETS[@]}"; do
  cargo build -p perch-ffi --lib --$PROFILE --target "$t"
done

# --- Fresh output dirs. ------------------------------------------------------
#
# Everything under $BUILD/$FW/$GEN is regenerated on every run, so wiping them
# first is what makes this script idempotent: a second run can't leave a stale
# slice, a stale header, or a stale Swift file behind from a previous
# invocation (e.g. one that used a different set of TARGETS).
rm -rf "$BUILD" "$FW" "$GEN"
mkdir -p "$BUILD/headers" "$GEN"

# --- Universal (or single-arch) static library. ------------------------------
#
# One XCFramework slice serves every Mac architecture we built for. `lipo
# -create` with a single input just copies it, so this works unmodified
# whether TARGETS has one entry (arm64-only fallback) or two (arm64 + x86_64).
LIB_PATHS=()
for t in "${TARGETS[@]}"; do
  LIB_PATHS+=("target/$t/$PROFILE/$LIB")
done
lipo -create "${LIB_PATHS[@]}" -output "$BUILD/$LIB"

# --- Swift bindings, from the compiled library (see header comment above). --
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$GEN" --swift-sources
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$BUILD/headers" --headers
# NOTE: deliberately NOT --xcframework. That flag emits a `framework module`,
# which expects headers under a `<Name>.framework/Headers` bundle reached via
# `-F`. SwiftPM's binaryTarget consumption instead exposes this xcframework
# slice's Headers/ dir with a plain `-I`, which only resolves a plain (non
# `framework`) module declaration - see task-7-report.md for how this was
# diagnosed. --module-name must match the generated Swift's
# `canImport(perch_ffiFFI)` / `import perch_ffiFFI` guard.
cargo run -p perch-ffi --bin uniffi-bindgen-swift -- "$BUILD/$LIB" "$BUILD/headers" \
  --modulemap --modulemap-filename module.modulemap --module-name perch_ffiFFI

xcodebuild -create-xcframework \
  -library "$BUILD/$LIB" -headers "$BUILD/headers" \
  -output "$FW"

echo "built $FW"
echo "slices: $(lipo -info "$BUILD/$LIB" 2>&1 | sed -e 's/^.*: //')"
echo "swift sources in $GEN:"; ls "$GEN"
