import AppKit
import Combine
import SwiftUI
import PerchFFI

/// The tray item and its menu. A real NSMenu: while it is open, macOS is tracking a
/// menu, which is the only thing that keeps the menu bar visible over a fullscreen app.
@MainActor
final class StatusItemController: NSObject, NSMenuDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private let engine: PerchEngine
    private var cancellables = Set<AnyCancellable>()
    /// The metrics the last rendered model chose; `nil` until one arrives.
    private var density: PerchDensityMetrics?
    /// One per session row currently drawn. An `NSMenuItem` does not retain
    /// its submenu's delegate, so these are held here for exactly as long as
    /// the rows they belong to are on screen, and replaced wholesale on the
    /// next render.
    private var submenus: [SessionSubmenu] = []

    /// Set by `AppDelegate`. The status item owns the menu, not the window's
    /// lifetime, so it just asks for the window to be shown.
    var onOpenWindow: (() -> Void)?
    /// Same idea, for the settings window.
    var onOpenSettings: (() -> Void)?

    init(engine: PerchEngine) {
        self.engine = engine
        super.init()
        menu.delegate = self
        statusItem.menu = menu
        statusItem.button?.imagePosition = .imageLeading
        engine.$model
            .receive(on: DispatchQueue.main)
            .sink { [weak self] in self?.render($0) }
            .store(in: &cancellables)
        render(nil)
    }

    func menuWillOpen(_ menu: NSMenu) {
        engine.refresh()
    }

    /// Every display choice below arrives on the model — which sections to
    /// draw, how tight a row is, whether a row carries its folder. None of it
    /// is read out of `Settings` here: two readers of one preference is how a
    /// window and a menu come to disagree about what is on screen.
    private func render(_ model: PopoverModel?) {
        statusItem.button?.title = model?.trayTitle ?? ""
        applyIcon(model)
        menu.removeAllItems()
        submenus.removeAll()
        // The density the *current* model asks for, handed to every card
        // hosted below. Before the first model there is none, so `add` falls
        // back to the environment default, which is the shipped one.
        density = model?.rowDensity.metrics

        guard let model else {
            add(EmptyCard(title: engine.startupError ?? "Starting…", detail: nil))
            menu.addItem(.separator())
            addOpenWindow()
            addSettings()
            menu.addItem(.separator())
            addQuit()
            return
        }

        add(StatsCard(stats: model.stats))
        if let err = model.error { add(EmptyCard(title: "Index unavailable", detail: err)) }
        menu.addItem(.separator())

        // The banner is Rust's decision, not a section: it reports that
        // something is blocked on the user, which stays true whether or not
        // they keep the waiting list on screen.
        if let banner = model.waitingBanner {
            add(EmptyCard(title: banner, detail: nil))
        }
        addLiveSections(model)
        // `show_recent == false` already leaves `recent` empty — the section
        // is not even queried — so there is nothing to re-filter here. The
        // flag decides only whether the header is drawn.
        if model.showRecent && !model.recent.isEmpty {
            menu.addItem(.separator())
            add(RecentCard(rows: model.recent))
        }
        menu.addItem(.separator())
        addOpenWindow()
        addSettings()
        menu.addItem(.separator())
        addQuit()
    }

    /// The two live sections, each drawn only if the user kept it. A session
    /// belongs to exactly one of them — waiting, or everything still open —
    /// so no session can fall through both toggles and become undrawable.
    private func addLiveSections(_ model: PopoverModel) {
        let waiting = model.live.filter { $0.status == .waiting }
        let working = model.live.filter { $0.status != .waiting }

        if model.live.isEmpty {
            add(EmptyCard(title: "No sessions running", detail: nil))
            return
        }
        if model.showWaiting && !waiting.isEmpty {
            add(SectionHeaderCard(title: "Waiting on you"))
            for row in waiting { addSession(row) }
        }
        if model.showWorking && !working.isEmpty {
            add(SectionHeaderCard(title: "Working"))
            for row in working { addSession(row) }
        }
    }

    /// One session row, plus the side menu of that session's own detail and
    /// actions. The row keeps its hosting view — which is why `SessionRowView`
    /// draws its own chevron, AppKit drawing no disclosure arrow for a
    /// view-backed item — and the submenu fills itself in when the pointer
    /// gets there, so its facts are read at the moment they are shown.
    private func addSession(_ row: SessionRow) {
        let item = add(SessionRowView(row: row))
        let submenu = SessionSubmenu(engine: engine, row: row, density: density ?? .comfortable)
        submenus.append(submenu)
        item.submenu = submenu.menu
    }

    /// Draw the glyph the model chose, dimmed when the data behind it has
    /// gone stale. `staleness` is a value whose presence *is* the flag, and
    /// its `label` is a finished sentence from Rust — rendered verbatim as
    /// the item's accessibility description, never re-phrased here.
    private func applyIcon(_ model: PopoverModel?) {
        guard let button = statusItem.button else { return }
        let stale = model?.staleness
        // No model yet means the engine has not emitted once (a startup
        // failure, in practice); the shipped default glyph is the honest
        // thing to draw until it does.
        let icon = model?.menuBarIcon ?? .bird
        let image = NSImage(
            systemSymbolName: icon.systemSymbolName,
            accessibilityDescription: stale?.label ?? "Perch"
        )
        // Template rendering is what makes the glyph follow the menu bar's
        // own light/dark appearance instead of staying one fixed colour.
        image?.isTemplate = true
        button.image = image
        // `appearsDisabled` fades image and title together without making
        // the item unclickable — the menu still opens while data is stale.
        button.appearsDisabled = stale != nil
    }

    /// Each card is its own hosting view, so the density has to be injected
    /// per root rather than once at a shared ancestor — there isn't one.
    @discardableResult
    private func add<V: View>(_ view: V) -> NSMenuItem {
        let item = NSMenuItem()
        let host = NSHostingView(rootView: view.environment(\.perchDensity, density ?? .comfortable))
        host.frame.size = host.fittingSize
        item.view = host
        menu.addItem(item)
        return item
    }

    private func addOpenWindow() {
        let item = NSMenuItem(title: "Open Perch", action: #selector(openWindow), keyEquivalent: "o")
        item.target = self
        menu.addItem(item)
    }

    private func addSettings() {
        let item = NSMenuItem(title: "Settings…", action: #selector(openSettings), keyEquivalent: ",")
        item.target = self
        menu.addItem(item)
    }

    private func addQuit() {
        menu.addItem(withTitle: "Quit Perch", action: #selector(quit), keyEquivalent: "q").target = self
    }

    @objc private func openWindow() { onOpenWindow?() }
    @objc private func openSettings() { onOpenSettings?() }
    @objc private func quit() { NSApp.terminate(nil) }
}

/// The SF Symbol each menu-bar variant draws as. This mapping lives in Swift
/// on purpose: what a glyph is *called* is the one genuinely macOS-specific
/// fact in the chain, and `MenuBarIcon` crosses the FFI as a variant so Rust
/// never has to know it. All four names were checked against this SDK.
private extension MenuBarIcon {
    var systemSymbolName: String {
        switch self {
        case .bird: "bird"
        case .binoculars: "binoculars"
        // The literal dot: a small filled circle, unadorned.
        case .dot: "circle.fill"
        // Level bars, not a hamburger — this sits beside a session count, so
        // `line.3.horizontal` would read as "menu" rather than "activity".
        case .bars: "chart.bar.fill"
        }
    }
}
