import AppKit
import Combine
import PerchFFI

/// The tray item and its menu. A real NSMenu: while it is open, macOS is tracking a
/// menu, which is the only thing that keeps the menu bar visible over a fullscreen app.
@MainActor
final class StatusItemController: NSObject, NSMenuDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private let engine: PerchEngine
    private var cancellables = Set<AnyCancellable>()

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
        menu.delegate = self
        statusItem.menu = menu
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "bird", accessibilityDescription: "Perch")
            button.imagePosition = .imageLeading
        }
        engine.$model
            .receive(on: DispatchQueue.main)
            .sink { [weak self] in self?.render($0) }
            .store(in: &cancellables)
        render(nil)
    }

    func menuWillOpen(_ menu: NSMenu) {
        engine.refresh()
    }

    private func render(_ model: PopoverModel?) {
        statusItem.button?.title = model?.trayTitle ?? ""
        menu.removeAllItems()
        // Cards arrive in Task 8; for now the menu is just Quit.
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit Perch", action: #selector(quit), keyEquivalent: "q").target = self
    }

    @objc private func quit() { NSApp.terminate(nil) }
}
