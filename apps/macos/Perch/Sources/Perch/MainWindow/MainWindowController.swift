import AppKit
import SwiftUI
import PerchFFI

/// Hosts the main window. The app is a menu-bar accessory, so it has no Dock
/// icon — but a window needs one to take focus and appear in Cmd-Tab, so the
/// activation policy flips to `.regular` while the window is up and back on close.
@MainActor
final class MainWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private let engine: PerchEngine

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
    }

    func show() {
        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            return
        }

        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 660),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        w.title = "Perch"
        w.titlebarAppearsTransparent = false
        w.minSize = NSSize(width: 820, height: 520)
        w.center()
        w.isReleasedWhenClosed = false
        w.delegate = self
        w.contentView = NSHostingView(rootView: MainWindowRoot(engine: engine))
        window = w

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Back to a pure menu-bar app: no Dock icon, no Cmd-Tab entry.
        NSApp.setActivationPolicy(.accessory)
    }
}

/// Placeholder content for the main window. Task 8 replaces this with the
/// sidebar and project list; this only proves the engine read reaches the UI.
struct MainWindowRoot: View {
    let engine: PerchEngine
    @State private var model: MainWindowModel?

    var body: some View {
        VStack(spacing: 12) {
            if let model {
                Text("\(model.projects.count) projects")
                    .font(.title2)
                if let error = model.error {
                    Text(error)
                        .foregroundStyle(.secondary)
                }
            } else {
                ProgressView("Loading…")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task {
            model = await engine.mainWindow()
        }
    }
}
