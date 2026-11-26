import AppKit
import PerchFFI

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
        status.onOpenSettings = { [weak self] in self?.mainWindow?.showSettings() }
        NSApp.mainMenu = mainMenu()
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

    /// Opens Settings in the main window, on `pane` when one is named.
    @MainActor
    func showSettings(pane: PaneId? = nil) {
        mainWindow?.showSettings(pane: pane)
    }

    @MainActor @objc private func openSettingsFromMenu() { showSettings() }

    /// The menu bar shown while the window is up. A menu-bar app still needs
    /// one: it is what makes ⌘, ⌘W, ⌘Q and the text-editing shortcuts work
    /// in the window.
    private func mainMenu() -> NSMenu {
        let bar = NSMenu()

        let app = NSMenu(title: "Perch")
        app.addItem(withTitle: "About Perch", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.addItem(.separator())
        let settings = app.addItem(withTitle: "Settings…", action: #selector(openSettingsFromMenu), keyEquivalent: ",")
        settings.target = self
        app.addItem(.separator())
        app.addItem(withTitle: "Hide Perch", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        app.addItem(withTitle: "Quit Perch", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        bar.addItem(withTitle: "Perch", action: nil, keyEquivalent: "").submenu = app

        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        bar.addItem(withTitle: "Edit", action: nil, keyEquivalent: "").submenu = edit

        let window = NSMenu(title: "Window")
        window.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        window.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        bar.addItem(withTitle: "Window", action: nil, keyEquivalent: "").submenu = window
        NSApp.windowsMenu = window

        return bar
    }

    func applicationWillTerminate(_ notification: Notification) {
        engine?.stop()
    }
}
