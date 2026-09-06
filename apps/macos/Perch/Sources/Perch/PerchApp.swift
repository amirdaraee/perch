import AppKit

@main
enum PerchApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var engine: PerchEngine?
    private var status: StatusItemController?
    private var mainWindow: MainWindowController?
    private var settingsWindow: SettingsWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Menu-bar app: no Dock icon, no app switcher entry.
        NSApp.setActivationPolicy(.accessory)
        let engine = PerchEngine()
        self.engine = engine
        let status = StatusItemController(engine: engine)
        self.status = status
        // Each window needs to know whether the *other* one is still open
        // before it flips the activation policy back to .accessory on
        // close — see the comment in `MainWindowController.windowWillClose`.
        let mainWindow = MainWindowController(
            engine: engine,
            isAnotherWindowOpen: { [weak self] in self?.settingsWindow?.isWindowOpen ?? false }
        )
        self.mainWindow = mainWindow
        let settingsWindow = SettingsWindowController(
            engine: engine,
            isAnotherWindowOpen: { [weak self] in self?.mainWindow?.isWindowOpen ?? false }
        )
        self.settingsWindow = settingsWindow
        status.onOpenWindow = { [weak self] in self?.mainWindow?.show() }
        status.onOpenSettings = { [weak self] in self?.settingsWindow?.show() }
        // Clicking a "waiting on you" notification opens the main window
        // with that session's project selected. `sessionId` isn't needed
        // here — see `Sidebar.swift`'s `reload()` for why `project` (the
        // session's directory name) is what identifies a project for this
        // purpose, and its own known limitation.
        engine.notifier.onClicked = { [weak self] _, project in
            self?.mainWindow?.show(selectingProjectNamed: project)
        }
        engine.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        engine?.stop()
    }
}
