.PHONY: framework build mcp run bundle clean

framework:
	../../../scripts/build-xcframework.sh

build:
	swift build -c release

# The MCP server ships inside the bundle, beside the app, so Settings can
# point Claude Code at it.
mcp:
	cargo build --release -p perch-mcp --manifest-path ../../../Cargo.toml

run: build
	./.build/release/Perch

# A minimal .app so LSUIElement and the bundle id apply.
bundle: build mcp
	rm -rf build/Perch.app
	mkdir -p build/Perch.app/Contents/MacOS build/Perch.app/Contents/Resources
	cp .build/release/Perch build/Perch.app/Contents/MacOS/Perch
	cp ../../../target/release/perch-mcp build/Perch.app/Contents/MacOS/perch-mcp
	cp Resources/Info.plist build/Perch.app/Contents/Info.plist
	cp Resources/Perch.icns build/Perch.app/Contents/Resources/Perch.icns
	# Ad-hoc sign *after* the Info.plist is in place, so the signature is
	# bound to it and the bundle identity is dev.perch.app rather than the
	# linker's bare "Perch". UserNotifications refuses to grant
	# authorization to a bundle whose identity is not bound to its
	# signature — it fails with UNErrorDomain 1 and never prompts.
	codesign --force --deep --sign - build/Perch.app
	@echo "open build/Perch.app"

clean:
	rm -rf .build build Frameworks Sources/PerchFFI
