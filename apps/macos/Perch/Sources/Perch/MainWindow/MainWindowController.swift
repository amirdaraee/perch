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

    /// `selectingProjectId` is a clicked "waiting on you" notification's
    /// `WaitingNotification.projectId` — the unique id of the project the
    /// alert was about, not its displayed directory name (two projects can
    /// share one). `MainWindowRoot` selects it once `reload()` has loaded the
    /// project list; `nil` — an ordinary open, or an alert whose directory
    /// the index has never seen — leaves whatever selection was already
    /// showing.
    func show(selectingProjectId id: Int64? = nil) {
        present(MainWindowRoot(engine: engine, refreshToken: nextToken(), selectProjectId: id, setTitle: setTitle))
    }

    /// Opens the window on Settings, on `pane` when one is named, otherwise
    /// on whichever pane was showing last.
    func showSettings(pane: PaneId? = nil) {
        if let pane {
            UserDefaults.standard.set(pane.key, forKey: SettingsRootView.paneDefaultsKey)
        }
        present(MainWindowRoot(engine: engine, refreshToken: nextToken(), openSettings: true, setTitle: setTitle))
    }

    private func nextToken() -> Int {
        refreshToken += 1
        return refreshToken
    }

    private var setTitle: @MainActor (String) -> Void {
        { [weak self] title in self?.window?.title = title }
    }

    private func present(_ root: MainWindowRoot) {
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
            (window.contentViewController as? NSHostingController<MainWindowRoot>)?.rootView = root
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
        w.isReleasedWhenClosed = false
        w.delegate = self
        window = w
        // A hosting *controller* with no sizing options, as Settings needed
        // when it was its own window: a bare hosting view sizes the window to
        // its content, and a settings pane's content is far taller than the
        // window, which hung the top of it out of sight.
        let host = NSHostingController(rootView: root)
        host.sizingOptions = []
        w.contentViewController = host
        // After the controller, which would otherwise resize the window.
        w.minSize = NSSize(width: 820, height: 520)
        w.setContentSize(NSSize(width: 1000, height: 660))
        w.center()

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Guarded so a sheet or panel sharing this delegate can't flip the
        // activation policy while the window itself stays open.
        guard notification.object as? NSWindow === window else { return }
        // Back to a pure menu-bar app: no Dock icon, no Cmd-Tab entry.
        NSApp.setActivationPolicy(.accessory)
    }
}
