.PHONY: framework build mcp run bundle release clean

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
# The per-arch output directory is asked of swift rather than hardcoded: it is
# .build/arm64-apple-macosx/release, not .build/release, and a wrong guess here
# would silently ship a stale binary from an earlier build.
build:
	@mkdir -p build
	@if [ -n "$(SWIFT_ARCHS)" ]; then \
	  paths=""; \
	  for a in $(SWIFT_ARCHS); do \
	    swift build -c release --arch $$a || exit 1; \
	    paths="$$paths $$(swift build -c release --arch $$a --show-bin-path)/Perch"; \
	  done; \
	  lipo -create $$paths -output build/Perch; \
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
	# Ad-hoc sign *after* the Info.plist is in place, so the signature is
	# bound to it and the bundle identity is dev.perch.app rather than the
	# linker's bare "Perch". UserNotifications refuses to grant
	# authorization to a bundle whose identity is not bound to its
	# signature — it fails with UNErrorDomain 1 and never prompts.
	codesign --force --deep --sign - build/Perch.app
	@echo "open build/Perch.app"

# The shippable artefact. `ditto`, never `zip`: zip does not preserve the
# symlink and permission structure of a .app, which invalidates the code
# signature — the app then fails to launch with a signature error rather than
# showing the Gatekeeper prompt the user could actually act on.
release:
	$(MAKE) bundle UNIVERSAL=1
	rm -f $(ZIP)
	ditto -c -k --keepParent build/Perch.app $(ZIP)
	@echo
	@lipo -info build/Perch.app/Contents/MacOS/Perch
	@lipo -info build/Perch.app/Contents/MacOS/perch-mcp
	@codesign --verify --deep --strict build/Perch.app && echo "signature ok"
	@shasum -a 256 $(ZIP)

clean:
	rm -rf .build build Frameworks Sources/PerchFFI
