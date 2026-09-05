.PHONY: framework build run bundle clean

framework:
	../../../scripts/build-xcframework.sh

build:
	swift build -c release

run: build
	./.build/release/Perch

# A minimal .app so LSUIElement and the bundle id apply.
bundle: build
	rm -rf build/Perch.app
	mkdir -p build/Perch.app/Contents/MacOS build/Perch.app/Contents/Resources
	cp .build/release/Perch build/Perch.app/Contents/MacOS/Perch
	cp Resources/Info.plist build/Perch.app/Contents/Info.plist
	@echo "open build/Perch.app"

clean:
	rm -rf .build build Frameworks Sources/PerchFFI
