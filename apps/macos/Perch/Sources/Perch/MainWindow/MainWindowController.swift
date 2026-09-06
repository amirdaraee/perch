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

    /// Reports whether some other top-level window (the settings window) is
    /// still open. Without this, closing whichever of the two regular
    /// windows closes last is fine, but closing one while the other is still
    /// up would wrongly flip the app back to `.accessory` — hiding the Dock
    /// icon out from under a window still on screen.
    private let isAnotherWindowOpen: () -> Bool

    /// Bumped every time `show()` (re)presents the window. `MainWindowRoot`
    /// keys its `.task` on this, so reopening the same `NSHostingView`
    /// reloads the project list instead of showing whatever was current the
    /// first time the view appeared — see `Sidebar.swift`.
    private var refreshToken = 0

    init(engine: PerchEngine, isAnotherWindowOpen: @escaping () -> Bool) {
        self.engine = engine
        self.isAnotherWindowOpen = isAnotherWindowOpen
        super.init()
    }

    /// Read by `SettingsWindowController` (via `AppDelegate`) so *it* knows
    /// not to drop the activation policy while this window is still open.
    /// `isVisible` alone is false for a *miniaturized* window, which is
    /// still very much open: minimize this one, close the other, and the app
    /// would drop to `.accessory` — leaving a window in the Dock with no Dock
    /// icon or Cmd-Tab entry left to restore it by, exactly what the guard
    /// this feeds exists to prevent.
    var isWindowOpen: Bool {
        guard let window else { return false }
        return window.isVisible || window.isMiniaturized
    }

    /// `selectingProjectId` is a clicked "waiting on you" notification's
    /// `WaitingNotification.projectId` — the unique id of the project the
    /// alert was about, not its displayed directory name (two projects can
    /// share one). `MainWindowRoot` selects it once `reload()` has loaded the
    /// project list; `nil` — an ordinary open, or an alert whose directory
    /// the index has never seen — leaves whatever selection was already
    /// showing.
    func show(selectingProjectId id: Int64? = nil) {
        refreshToken += 1

        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            // Otherwise picking "Open Perch" while the window is minimized
            // brings the app frontmost with nothing visibly happening — the
            // window is still sitting in the Dock, miniaturized.
            if window.isMiniaturized {
                window.deminiaturize(nil)
            }
            window.makeKeyAndOrderFront(nil)
            (window.contentView as? NSHostingView<MainWindowRoot>)?.rootView =
                MainWindowRoot(engine: engine, refreshToken: refreshToken, selectProjectId: id)
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
        w.contentView = NSHostingView(
            rootView: MainWindowRoot(engine: engine, refreshToken: refreshToken, selectProjectId: id)
        )
        window = w

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Guarded even though there is exactly one window/delegate here:
        // without it, a second window sharing this delegate would flip the
        // activation policy back to .accessory when *it* closed, even while
        // this controller's own window was still open.
        guard notification.object as? NSWindow === window else { return }
        // Back to a pure menu-bar app: no Dock icon, no Cmd-Tab entry — but
        // only if the settings window isn't still up; otherwise that window
        // would be left on screen with no Dock icon or Cmd-Tab entry to
        // reach it by.
        guard !isAnotherWindowOpen() else { return }
        NSApp.setActivationPolicy(.accessory)
    }
}
