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

    /// Bumped every time `show()` (re)presents the window. `MainWindowRoot`
    /// keys its `.task` on this, so reopening the same `NSHostingView`
    /// reloads the project list instead of showing whatever was current the
    /// first time the view appeared — see `Sidebar.swift`.
    private var refreshToken = 0

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
    }

    func show() {
        refreshToken += 1

        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            (window.contentView as? NSHostingView<MainWindowRoot>)?.rootView =
                MainWindowRoot(engine: engine, refreshToken: refreshToken)
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
        w.contentView = NSHostingView(rootView: MainWindowRoot(engine: engine, refreshToken: refreshToken))
        window = w

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Guarded even though there is exactly one window/delegate today:
        // without it, a second window sharing this delegate would flip the
        // activation policy back to .accessory when *it* closed, even while
        // this controller's own window was still open.
        guard notification.object as? NSWindow === window else { return }
        // Back to a pure menu-bar app: no Dock icon, no Cmd-Tab entry.
        NSApp.setActivationPolicy(.accessory)
    }
}
