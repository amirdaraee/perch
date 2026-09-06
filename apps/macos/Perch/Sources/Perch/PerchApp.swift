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
        // here: the project id Rust put in the payload is what identifies
        // the project, and a `nil` one opens the window with whatever was
        // already selected rather than selecting a guess.
        engine.notifier.onClicked = { [weak self] _, projectId in
            self?.mainWindow?.show(selectingProjectId: projectId)
        }
        engine.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        engine?.stop()
    }
}
