.PHONY: framework build mcp run bundle sign package release notarize clean

ROOT := ../../..

# Set UNIVERSAL=1 to build for arm64 AND x86_64 and lipo the slices together.
# The release workflow does; a plain `make` builds for the host alone, which is
# about twice as fast and all you need while iterating.
#
# This matters more than it looks: GitHub's macOS runners are Apple Silicon, so
# a release built without it is silently arm64-only and simply fails to launch
# on an Intel Mac. `scripts/build-xcframework.sh` already lipos the Rust static
# library; this is the other half — the Swift executable and perch-mcp.
UNIVERSAL ?=

ifdef UNIVERSAL
  SWIFT_ARCHS := arm64 x86_64
  MCP_TARGETS := aarch64-apple-darwin x86_64-apple-darwin
else
  SWIFT_ARCHS :=
  MCP_TARGETS :=
endif

# Who signs the bundle. `-` is an ad-hoc signature, which is what every build
# gets until there is a Developer ID: it runs, but Gatekeeper makes the user
# click "Open Anyway". The release workflow sets this to the Developer ID
# Application identity once the signing secrets exist.
SIGN_IDENTITY ?= -

# The hardened runtime is on for every build, ad-hoc included. Notarization
# refuses anything without it, and turning it on only for real releases would
# mean the first build to run under it is the one shipped to users. A secure
# timestamp needs a real identity (and Apple's timestamp server), so ad-hoc
# builds go without.
CODESIGN := codesign --force --options runtime
ifneq ($(SIGN_IDENTITY),-)
  CODESIGN += --timestamp
endif

# Matches Package.swift and LSMinimumSystemVersion. Exported so the Rust and C
# code in perch-mcp is built for it too, not for the build machine's macOS.
export MACOSX_DEPLOYMENT_TARGET := 15.0

VERSION := $(shell /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' Resources/Info.plist)
ZIP := build/Perch-$(VERSION)-macos.zip

framework:
	$(ROOT)/scripts/build-xcframework.sh

# Builds one binary at build/Perch, universal or host-only, so `bundle` has a
# single path to copy either way.
#
# Note the loop: `swift build --arch arm64 --arch x86_64` in ONE invocation
# fails here with `ld: library not found for -lperch_ffi`, even though the
# xcframework declares (and genuinely contains) both slices — SwiftPM does not
# resolve a binaryTarget xcframework in its multi-arch mode. Each arch built
# alone links fine, so we build them separately and lipo the executables, the
# same shape build-xcframework.sh uses for the static library.
#
# The per-arch output directory is asked of swift rather than hardcoded, and
# each binary is copied out *before* the next arch builds. Older SwiftPM puts
# each arch in its own directory (.build/arm64-apple-macosx/release); Swift 6.4
# uses one shared .build/out/Products/Release for both, so the second build
# overwrites the first and lipo is handed two x86_64 copies.
build:
	@mkdir -p build
	@if [ -n "$(SWIFT_ARCHS)" ]; then \
	  paths=""; \
	  for a in $(SWIFT_ARCHS); do \
	    swift build -c release --arch $$a || exit 1; \
	    cp "$$(swift build -c release --arch $$a --show-bin-path)/Perch" build/Perch-$$a || exit 1; \
	    paths="$$paths build/Perch-$$a"; \
	  done; \
	  lipo -create $$paths -output build/Perch || exit 1; \
	  rm -f $$paths; \
	else \
	  swift build -c release || exit 1; \
	  cp "$$(swift build -c release --show-bin-path)/Perch" build/Perch; \
	fi

# The MCP server ships inside the bundle, beside the app, so Settings can
# point Claude Code at it. It has to match the app's architectures: an
# arm64-only perch-mcp inside a universal app leaves Intel users with an app
# that runs and an MCP server that doesn't.
mcp:
	@mkdir -p build
	@if [ -n "$(MCP_TARGETS)" ]; then \
	  for t in $(MCP_TARGETS); do \
	    rustup target add $$t >/dev/null 2>&1 || true; \
	    cargo build --release -p perch-mcp --manifest-path $(ROOT)/Cargo.toml --target $$t || exit 1; \
	  done; \
	  lipo -create $(foreach t,$(MCP_TARGETS),$(ROOT)/target/$(t)/release/perch-mcp) -output build/perch-mcp; \
	else \
	  cargo build --release -p perch-mcp --manifest-path $(ROOT)/Cargo.toml || exit 1; \
	  cp $(ROOT)/target/release/perch-mcp build/perch-mcp; \
	fi

run: build
	./build/Perch

# A minimal .app so LSUIElement and the bundle id apply.
bundle: build mcp
	rm -rf build/Perch.app
	mkdir -p build/Perch.app/Contents/MacOS build/Perch.app/Contents/Resources
	cp build/Perch build/Perch.app/Contents/MacOS/Perch
	cp build/perch-mcp build/Perch.app/Contents/MacOS/perch-mcp
	cp Resources/Info.plist build/Perch.app/Contents/Info.plist
	cp Resources/Perch.icns build/Perch.app/Contents/Resources/Perch.icns
	# Sign *after* the Info.plist is in place, so the signature is bound to
	# it and the bundle identity is dev.perch.app rather than the linker's
	# bare "Perch". UserNotifications refuses to grant authorization to a
	# bundle whose identity is not bound to its signature — it fails with
	# UNErrorDomain 1 and never prompts.
	$(MAKE) sign
	@echo "open build/Perch.app"

# Signs the bundle that is already in build/, without building anything. The
# release workflow builds first and only then unlocks the Developer ID, so no
# dependency's build script ever runs while the signing key is usable.
#
# Inside out, not `--deep`: the nested perch-mcp is signed first with its own
# identifier, then the bundle around it. `--deep` applies one set of options to
# everything and is deprecated for Developer ID signing.
sign:
	$(CODESIGN) --sign "$(SIGN_IDENTITY)" --identifier dev.perch.app.mcp \
	  build/Perch.app/Contents/MacOS/perch-mcp
	$(CODESIGN) --sign "$(SIGN_IDENTITY)" build/Perch.app

# The shippable artefact. `ditto`, never `zip`: zip does not preserve the
# symlink and permission structure of a .app, which invalidates the code
# signature — the app then fails to launch with a signature error rather than
# showing the Gatekeeper prompt the user could actually act on.
release:
	$(MAKE) bundle UNIVERSAL=1
	$(MAKE) package

# Zips and checks whatever is in build/Perch.app, as signed. Split from
# `release` so a re-signed bundle can be packaged without being rebuilt.
package:
	rm -f $(ZIP)
	ditto -c -k --keepParent build/Perch.app $(ZIP)
	@echo
	@lipo -info build/Perch.app/Contents/MacOS/Perch
	@lipo -info build/Perch.app/Contents/MacOS/perch-mcp
	@codesign --verify --deep --strict build/Perch.app && echo "signature ok"
	@for bin in Perch perch-mcp; do \
	  codesign -d --verbose=2 build/Perch.app/Contents/MacOS/$$bin 2>&1 \
	    | grep -q 'flags=.*runtime' \
	    || { echo "$$bin is not signed with the hardened runtime" >&2; exit 1; }; \
	done; echo "hardened runtime ok"
	@shasum -a 256 $(ZIP)

# Needs a Developer ID-signed bundle (`make sign package SIGN_IDENTITY=...`)
# and the NOTARY_* variables described in scripts/notarize.sh. Replaces $(ZIP)
# with one containing the stapled app.
notarize:
	$(ROOT)/scripts/notarize.sh build/Perch.app $(ZIP)

clean:
	rm -rf .build build Frameworks Sources/PerchFFI
