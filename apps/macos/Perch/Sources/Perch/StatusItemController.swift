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
    /// One per session, keyed by session id. An `NSMenuItem` holds its target
    /// and its submenu's delegate weakly, so the controller has to be held
    /// here or a submenu would go inert the moment it was built.
    private var submenus: [String: SessionSubmenu] = [:]

    /// The newest model. Most ticks only store this: the menu is closed nearly
    /// all the time, and nobody can see a menu being rebuilt.
    private var latest: PopoverModel?
    private var isOpen = false
    /// The items no longer match `latest`, so rebuild them before the next open.
    private var needsRebuild = true
    /// The items that can change while the menu is open, updated in place.
    /// Removing an item would close the submenu hanging off it under the
    /// pointer, which is why the menu is never rebuilt while it is open.
    private var rowItems: [String: NSMenuItem] = [:]
    private var summaryItem: NSMenuItem?
    private var statsItem: NSMenuItem?

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
            .sink { [weak self] in self?.receive($0) }
            .store(in: &cancellables)
        // AppKit won't open an empty menu, so it can't wait for the first open.
        rebuild()
    }

    /// Sent just before the menu shows, which is when its items have to be
    /// right, and the one moment they can safely all be replaced.
    func menuNeedsUpdate(_ menu: NSMenu) {
        if needsRebuild { rebuild() }
    }

    func menuWillOpen(_ menu: NSMenu) {
        isOpen = true
        engine.refresh()
    }

    func menuDidClose(_ menu: NSMenu) {
        isOpen = false
    }

    private func receive(_ model: PopoverModel?) {
        latest = model
        needsRebuild = true
        statusItem.button?.title = model?.trayTitle ?? ""
        applyIcon(model)
        if isOpen, let model { updateInPlace(model) }
    }

    /// Refresh what's on screen without adding or removing anything. A session
    /// that started or ended since the menu opened shows up on the next open.
    private func updateInPlace(_ model: PopoverModel) {
        summaryItem?.attributedTitle = lineTitle(model.summaryLine, primary: true)
        statsItem?.attributedTitle = lineTitle(model.statsLine, primary: false)
        for row in model.live {
            guard let item = rowItems[row.id] else { continue }
            style(item, for: row, density: model.rowDensity.metrics)
        }
    }

    /// Every display choice below arrives on the model — which sections to
    /// draw, how tight a row is, whether a row carries its folder. None of it
    /// is read out of `Settings` here: two readers of one preference is how a
    /// window and a menu come to disagree about what is on screen.
    private func rebuild() {
        let model = latest
        needsRebuild = false
        // Detach every submenu from the item that carried it before the items
        // go: an `NSMenu` may belong to one menu item at a time, and these
        // menus outlive the items, being reattached to the new rows.
        for item in menu.items { item.submenu = nil }
        menu.removeAllItems()
        rowItems = [:]
        summaryItem = nil
        statsItem = nil
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

        if let err = model.error { add(EmptyCard(title: "Index unavailable", detail: err)) }
        if model.summaryLine.isEmpty {
            add(EmptyCard(title: "No sessions running", detail: nil))
        } else {
            summaryItem = addLine(model.summaryLine, primary: true)
        }
        addSessions(model)
        // `show_recent == false` already leaves `recent` empty — the section
        // is not even queried — so there is nothing to re-filter here. The
        // flag decides only whether the header is drawn.
        if model.showRecent && !model.recent.isEmpty {
            menu.addItem(.separator())
            add(RecentCard(rows: model.recent))
        }
        if !model.statsLine.isEmpty {
            menu.addItem(.separator())
            statsItem = addLine(model.statsLine, primary: false)
        }
        menu.addItem(.separator())
        addOpenWindow()
        addSettings()
        menu.addItem(.separator())
        addQuit()
    }

    /// One row per session, most urgent first (Rust's order), each drawn only if
    /// the user kept its section. A session belongs to exactly one of them —
    /// waiting, or everything still open — so no session can fall through both
    /// toggles and become undrawable.
    private func addSessions(_ model: PopoverModel) {
        // A session that is no longer live has no row to hang a submenu off,
        // so its controller goes.
        let liveIds = Set(model.live.map(\.id))
        submenus = submenus.filter { liveIds.contains($0.key) }

        let shown = model.live.filter { $0.status == .waiting ? model.showWaiting : model.showWorking }
        if shown.isEmpty { return }
        menu.addItem(.separator())
        for row in shown { addSession(row, density: model.rowDensity.metrics) }
    }

    /// One session row, plus its side menu. A plain menu item rather than a
    /// hosted view, which gets it AppKit's own highlight, disclosure arrow and
    /// hover behaviour, and lets it change while the menu is open. The submenu
    /// fills itself in when the pointer gets there, so its facts are read at
    /// the moment they are shown.
    private func addSession(_ row: SessionRow, density metrics: PerchDensityMetrics) {
        let item = NSMenuItem()
        style(item, for: row, density: metrics)
        // Reused when this session already had one. Its contents are composed
        // afresh on every open, so nothing stale is kept — only the pid and the
        // density it draws at, which are updated here.
        let submenu = submenus[row.id] ?? SessionSubmenu(engine: engine, row: row, density: metrics)
        submenu.adopt(row: row, density: metrics)
        submenus[row.id] = submenu
        item.submenu = submenu.menu
        menu.addItem(item)
        rowItems[row.id] = item
    }

    private func style(_ item: NSMenuItem, for row: SessionRow, density metrics: PerchDensityMetrics) {
        item.image = Self.dot(for: row.status)
        item.attributedTitle = rowTitle(row, density: metrics)
    }

    /// `name    Working 12s`, the state lined up at the right edge, with the
    /// folder underneath if the user asked for it. Every word is Rust's.
    private func rowTitle(_ row: SessionRow, density metrics: PerchDensityMetrics) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.tabStops = [NSTextTab(textAlignment: .right, location: RowMetrics.stateEdge)]
        paragraph.paragraphSpacingBefore = metrics.row / 2
        paragraph.paragraphSpacing = metrics.row / 2

        let title = NSMutableAttributedString(
            string: row.menuName,
            attributes: [.font: NSFont.menuFont(ofSize: 0), .paragraphStyle: paragraph]
        )
        title.append(NSAttributedString(
            string: "\t" + row.glance,
            attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular),
                .foregroundColor: row.status == .waiting ? NSColor(Color.perchWaiting) : NSColor.secondaryLabelColor,
                .paragraphStyle: paragraph,
            ]
        ))
        // Blank means the user turned "Show project folder" off.
        if !row.folder.isEmpty {
            title.append(NSAttributedString(
                string: "\n" + row.folder,
                attributes: [
                    .font: NSFont.menuFont(ofSize: NSFont.smallSystemFontSize),
                    .foregroundColor: NSColor.tertiaryLabelColor,
                ]
            ))
        }
        return title
    }

    /// A line of plain information. Disabled, because there's nothing to do with
    /// it, with its colour set so it doesn't get drawn dimmed as well.
    private func addLine(_ text: String, primary: Bool) -> NSMenuItem {
        let item = NSMenuItem()
        item.attributedTitle = lineTitle(text, primary: primary)
        item.isEnabled = false
        menu.addItem(item)
        return item
    }

    private func lineTitle(_ text: String, primary: Bool) -> NSAttributedString {
        NSAttributedString(
            string: text,
            attributes: primary
                ? [
                    .font: NSFont.menuFont(ofSize: 0).withWeight(.semibold),
                    .foregroundColor: NSColor.labelColor,
                ]
                : [
                    .font: NSFont.menuFont(ofSize: NSFont.smallSystemFontSize),
                    .foregroundColor: NSColor.secondaryLabelColor,
                ]
        )
    }

    /// The status dot. Drawn on demand so it follows the appearance and matches
    /// the colours used everywhere else in the app.
    private static func dot(for status: Status) -> NSImage {
        let color: NSColor = switch status {
        case .waiting: NSColor(Color.perchWaiting)
        case .working: NSColor(Color.perchWorking)
        case .idle: NSColor(Color.perchWorking.opacity(0.55))
        case .background: NSColor(Color.perchBackground)
        }
        return NSImage(size: NSSize(width: 8, height: 8), flipped: false) { rect in
            color.setFill()
            NSBezierPath(ovalIn: rect).fill()
            return true
        }
    }

    private enum RowMetrics {
        /// Where the state lines up: enough for a full-length name beside it.
        static let stateEdge: CGFloat = 370
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

private extension NSFont {
    func withWeight(_ weight: NSFont.Weight) -> NSFont {
        let traits = [NSFontDescriptor.TraitKey.weight: weight]
        let descriptor = fontDescriptor.addingAttributes([.traits: traits])
        return NSFont(descriptor: descriptor, size: pointSize) ?? self
    }
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
