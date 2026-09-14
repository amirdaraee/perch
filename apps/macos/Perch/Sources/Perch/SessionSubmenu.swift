import AppKit
import SwiftUI
import PerchFFI

/// The side menu that hangs off one session row: that session's own facts,
/// and the four things a user can do to it.
///
/// One of these per drawn row, owned by `StatusItemController` for exactly as
/// long as the row it belongs to is on screen. It is the menu's delegate, so
/// AppKit tells it when the pointer has arrived and the contents are needed.
@MainActor
final class SessionSubmenu: NSObject, NSMenuDelegate {
    let menu = NSMenu()

    private let engine: PerchEngine
    private let sessionId: String
    private let pid: Int32
    private let density: PerchDensityMetrics

    /// What the last `menuNeedsUpdate` composed, kept because the click comes
    /// afterwards and the handlers need the id and path Rust handed over.
    private var model: SessionMenu?

    init(engine: PerchEngine, row: SessionRow, density: PerchDensityMetrics) {
        self.engine = engine
        self.sessionId = row.id
        self.pid = row.pid
        self.density = density
        super.init()
        menu.delegate = self
        // Rust decides what is available and why. AppKit's automatic
        // enable/disable pass looks for a target that responds to each
        // item's action and would re-enable every one of them the moment the
        // menu opened, overruling that decision.
        menu.autoenablesItems = false
        // An empty menu is not a menu AppKit will open, and this one has no
        // contents until the pointer arrives. A single empty disabled item is
        // enough to make it real, and `menuNeedsUpdate` clears it before it
        // can ever be seen.
        menu.addItem(placeholder())
    }

    /// Sent immediately before the submenu is displayed, which is exactly
    /// when its facts should be read: the point of hovering a row is to see
    /// what is true now, and the session's folder may have been deleted since
    /// the popover opened.
    ///
    /// Synchronous on purpose — AppKit is between frames here and cannot be
    /// handed a promise. The work behind it is one directory read and a
    /// couple of indexed lookups, the same shape of read the popover already
    /// performs on every poll.
    func menuNeedsUpdate(_ menu: NSMenu) {
        // Resolved here and named by Rust; Focus resolves again when it is
        // actually clicked, so nothing about this is remembered.
        let owner = SessionFocus.owningApplication(of: pid)
        model = engine.sessionMenu(sessionId: sessionId, owningApp: owner?.localizedName)
        menu.removeAllItems()
        guard let model else {
            // The session ended between being drawn and being pointed at.
            // Rust says so by returning nothing; the honest drawing of that
            // is an empty menu, not a menu of facts that are no longer true.
            menu.addItem(placeholder())
            return
        }

        let host = NSHostingView(
            rootView: SessionDetailCard(rows: model.detail).environment(\.perchDensity, density)
        )
        host.frame.size = host.fittingSize
        let detail = NSMenuItem()
        detail.view = host
        menu.addItem(detail)
        menu.addItem(.separator())

        for action in model.actions {
            menu.addItem(item(for: action))
            if let reason = action.disabledReason {
                menu.addItem(explanation(reason))
            }
        }
    }

    // MARK: - Items

    private func placeholder() -> NSMenuItem {
        let item = NSMenuItem()
        item.isEnabled = false
        return item
    }

    private func item(for action: MenuAction) -> NSMenuItem {
        let item = NSMenuItem(title: action.title, action: #selector(performAction(_:)), keyEquivalent: "")
        item.target = self
        item.isEnabled = action.enabled
        item.representedObject = action.kind
        item.toolTip = action.disabledReason
        return item
    }

    /// A disabled action says why, in smaller secondary type under the item
    /// it explains — a dead menu item with no explanation is the thing this
    /// avoids. The sentence is Rust's; only its size and indent are decided
    /// here.
    private func explanation(_ reason: String) -> NSMenuItem {
        let item = NSMenuItem()
        item.attributedTitle = NSAttributedString(
            string: reason,
            attributes: [
                .font: NSFont.menuFont(ofSize: NSFont.smallSystemFontSize),
                .foregroundColor: NSColor.secondaryLabelColor,
            ]
        )
        item.isEnabled = false
        item.indentationLevel = 1
        return item
    }

    // MARK: - Doing the thing

    @objc private func performAction(_ sender: NSMenuItem) {
        guard let kind = sender.representedObject as? MenuActionKind, let model else { return }
        switch kind {
        case .resume:
            Task {
                guard
                    let command = await engine.resumeCommand(
                        sessionId: model.sessionId, cwd: model.folderPath
                    ),
                    let terminal = await engine.preferredTerminal()
                else { return }
                try? Launcher.run(command, terminal: terminal)
            }
        case .focus:
            // Re-resolved rather than reusing the application found while
            // building the menu: a terminal can quit between the submenu
            // opening and the item being clicked.
            SessionFocus.activateApplication(of: pid)
        case .revealFolder:
            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: model.folderPath)])
        case .copySessionId:
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(model.sessionId, forType: .string)
        }
    }
}
