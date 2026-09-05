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

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Menu-bar app: no Dock icon, no app switcher entry.
        NSApp.setActivationPolicy(.accessory)
        let engine = PerchEngine()
        self.engine = engine
        let status = StatusItemController(engine: engine)
        self.status = status
        let mainWindow = MainWindowController(engine: engine)
        self.mainWindow = mainWindow
        status.onOpenWindow = { [weak self] in self?.mainWindow?.show() }
        engine.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        engine?.stop()
    }
}
