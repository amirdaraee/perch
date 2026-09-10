import AppKit
import SwiftUI
import UserNotifications
import PerchFFI

/// Hosts the settings window, following `MainWindowController`'s established
/// pattern: `isReleasedWhenClosed = false`, a `notification.object === window`
/// guard in `windowWillClose`, and the activation-policy flip so the window
/// can take focus while the app stays a menu-bar app.
///
/// One addition over `MainWindowController`: this app can now have *two*
/// regular windows open at once (main + settings), so closing either one
/// must check whether the other is still up before dropping the app back to
/// `.accessory` — see `isAnotherWindowOpen`.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    private let engine: PerchEngine
    private let isAnotherWindowOpen: () -> Bool

    /// Bumped every time `show()` (re)presents the window, the same trick
    /// `MainWindowController` uses to make `.task(id:)` reload on reopen
    /// rather than showing whatever was current when the view first
    /// appeared.
    private var refreshToken = 0

    init(engine: PerchEngine, isAnotherWindowOpen: @escaping () -> Bool) {
        self.engine = engine
        self.isAnotherWindowOpen = isAnotherWindowOpen
        super.init()
    }

    /// Read by `MainWindowController` (via `AppDelegate`) for the same reason
    /// in reverse.
    /// `isVisible` alone is false for a *miniaturized* window, which is
    /// still very much open: minimize this one, close the other, and the app
    /// would drop to `.accessory` — leaving a window in the Dock with no Dock
    /// icon or Cmd-Tab entry left to restore it by, exactly what the guard
    /// this feeds exists to prevent.
    var isWindowOpen: Bool {
        guard let window else { return false }
        return window.isVisible || window.isMiniaturized
    }

    func show() {
        refreshToken += 1

        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            if window.isMiniaturized {
                window.deminiaturize(nil)
            }
            window.makeKeyAndOrderFront(nil)
            (window.contentView as? NSHostingView<SettingsRootView>)?.rootView = rootView()
            return
        }

        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 880, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        w.title = "Settings"
        w.setFrameAutosaveName("PerchSettingsWindow")
        w.center()
        w.minSize = NSSize(width: 720, height: 480)
        w.isReleasedWhenClosed = false
        w.delegate = self
        window = w
        w.contentView = NSHostingView(rootView: rootView())

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    /// The window's title names the pane the user is looking at, so the
    /// title bar says where they are rather than repeating the app's name at
    /// them. SwiftUI's `navigationTitle` does not reach an `NSWindow` this
    /// view was merely dropped into, so the view hands the string back here.
    private func rootView() -> SettingsRootView {
        SettingsRootView(
            engine: engine,
            refreshToken: refreshToken,
            setTitle: { [weak self] title in self?.window?.title = title }
        )
    }

    func windowWillClose(_ notification: Notification) {
        guard notification.object as? NSWindow === window else { return }
        // Back to a pure menu-bar app — but only if the main window isn't
        // still up; otherwise that window would be left on screen with no
        // Dock icon or Cmd-Tab entry to reach it by.
        guard !isAnotherWindowOpen() else { return }
        NSApp.setActivationPolicy(.accessory)
    }
}

/// The settings window's content: a sidebar of panes over a detail side that
/// draws whichever one is selected.
///
/// **The sidebar is the schema's, not Swift's.** Every pane, its title, its
/// icon, its attention phrase and all of its rows arrive from
/// `settingsSchema(query:)`; a hardcoded list here would be a second copy of
/// all of it, and the copy is what goes stale. Search is Rust's too — this
/// view sends keystrokes and draws what comes back.
///
/// What makes this window Perch's rather than a generic preferences sheet is
/// the panel at the top of the detail side: each pane previews its own effect
/// against the user's real index — `12 Active · 19 Recent · 4 Archived` — and
/// the sidebar flags, in a short phrase, any pane whose current configuration
/// is costing something right now. Both strings are composed in Rust from
/// real counts; neither is ever a fabricated zero.
struct SettingsRootView: View {
    let engine: PerchEngine
    let refreshToken: Int
    let setTitle: @MainActor (String) -> Void

    /// Observed, not merely read: `Notifier` re-reads macOS's authorization
    /// on every delivery, so a permission granted or revoked while this
    /// window is open — or a delivery macOS refused — repaints the pane
    /// below without waiting for a reopen.
    @ObservedObject private var notifier: Notifier

    @MainActor
    init(engine: PerchEngine, refreshToken: Int, setTitle: @escaping @MainActor (String) -> Void) {
        self.engine = engine
        self.refreshToken = refreshToken
        self.setTitle = setTitle
        _notifier = ObservedObject(wrappedValue: engine.notifier)
    }

    /// The schema as Rust last returned it — filtered by `query` when one is
    /// typed. This is the single source every control's `get` reads from.
    @State private var panes: [SettingsPane] = []
    /// Whatever `Settings::validated` had to change on the way to disk: a
    /// number clamped to its range, a folder that was not a folder. Returned
    /// by the write that caused it, so a clamp is never silent.
    @State private var notes: [String] = []
    /// Set only when the schema read itself fails — i.e. the engine isn't
    /// running. Rendered only while there are no panes to draw.
    @State private var loadError: String?
    /// Set when a write fails, independent of `loadError`. Rendered
    /// unconditionally, whether or not panes have loaded — mirrors
    /// `ProjectDetailPane.actionError`, which exists for exactly this
    /// reason: a post-load failure must never be gated behind the "still
    /// loading" branch, or it silently disappears the moment a model exists.
    @State private var actionError: String?

    @State private var query: String = ""
    /// Cancels the previous keystroke's fetch so a fast typist's results
    /// cannot land out of order.
    @State private var searchTask: Task<Void, Never>?

    /// Chains queued writes so a second control's edit always builds on the
    /// schema the first write returned, not on the pre-edit one both would
    /// otherwise read if their `setSetting` calls overlapped.
    @State private var pendingSave: Task<Void, Never>?

    /// Per-viewer window state, not settings: neither belongs in
    /// `config.toml`, which is the user's own hand-editable file.
    @AppStorage("settings.selectedPane") private var selectedPaneKey: String = "general"
    @AppStorage("settings.sidebarWidth") private var sidebarWidth: Double = 228
    /// The remembered width, snapshotted once per presentation. Feeding the
    /// live `sidebarWidth` straight back into `ideal:` would make the column
    /// argue with the drag that is setting it.
    @State private var idealSidebarWidth: Double?

    var body: some View {
        NavigationSplitView {
            sidebar
                .navigationSplitViewColumnWidth(
                    min: 200, ideal: idealSidebarWidth ?? sidebarWidth, max: 320
                )
        } detail: {
            detail
        }
        .frame(minWidth: 720, minHeight: 480)
        .task(id: refreshToken) { await load() }
        .onChange(of: query) { _, q in search(q) }
        .onChange(of: currentPane?.title) { _, title in
            setTitle(title ?? "Settings")
        }
        .onAppear {
            setTitle(currentPane?.title ?? "Settings")
            if idealSidebarWidth == nil { idealSidebarWidth = sidebarWidth }
        }
    }

    // MARK: - Sidebar

    @ViewBuilder
    private var sidebar: some View {
        VStack(spacing: 0) {
            searchField
            if panes.isEmpty && !query.isEmpty {
                // Never an empty sidebar: an empty list and a broken window
                // look exactly alike, and the user has no way to tell which
                // one they are looking at.
                VStack(alignment: .leading, spacing: 8) {
                    Text("No setting matches “\(query)”.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("Clear search") { query = "" }
                        .buttonStyle(.link)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 14)
                Spacer()
            } else {
                List(selection: selection) {
                    ForEach(panes, id: \.id.key) { pane in
                        sidebarRow(pane).tag(pane.id.key)
                    }
                }
                .listStyle(.sidebar)
            }
        }
        .background(SidebarWidthReporter(width: $sidebarWidth))
    }

    private func sidebarRow(_ pane: SettingsPane) -> some View {
        HStack(spacing: 8) {
            // Monochrome by choice. Coloured icon tiles solve a problem
            // Perch does not have — nine panes of one app do not need
            // colour to be told apart, and the one accent here is spent on
            // selection and on the attention phrases instead.
            Image(systemName: symbolName(pane.icon))
                .font(.system(size: 13))
                .frame(width: 18, alignment: .center)
            Text(pane.title)
                .lineLimit(1)
            Spacer(minLength: 4)
            // Drawn only when Rust says something is actually costing the
            // user something. `nil` draws nothing at all — never a zero, and
            // never a placeholder, or the badge stops being read.
            if let attention = pane.attention {
                Text(attention)
                    .font(.caption2)
                    .foregroundStyle(Color.accentColor)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Capsule().fill(Color.accentColor.opacity(0.14)))
                    .fixedSize()
            }
        }
        .padding(.vertical, 2)
    }

    private var searchField: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .font(.system(size: 12))
            TextField("Search", text: $query)
                .textFieldStyle(.plain)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Color(nsColor: .textBackgroundColor))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.10))
        )
        .padding(.horizontal, 10)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }

    // MARK: - Detail

    @ViewBuilder
    private var detail: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let loadError, panes.isEmpty {
                banner(loadError, color: .red)
            }
            // Unconditional: a write that failed must reach the user whether
            // or not a schema is on screen behind it.
            if let actionError {
                banner(actionError, color: .red)
            }
            if !notes.isEmpty {
                banner(notes.joined(separator: "\n"), color: .orange)
            }

            if let pane = currentPane {
                if let preview = pane.preview {
                    previewPanel(preview)
                }
                if pane.id == .notifications {
                    notificationPermissionNotes
                }
                paneBody(pane)
            } else if !query.isEmpty {
                emptyDetail
            } else if loadError == nil {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Spacer()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// Perch's differentiator, and so the first thing on the detail side
    /// rather than something buried under the controls: what this pane's
    /// settings are doing right now, in the user's own numbers. Every string
    /// is composed in Rust.
    private func previewPanel(_ preview: PanePreview) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("Right now")
                .font(.caption2.weight(.semibold))
                .textCase(.uppercase)
                .foregroundStyle(.secondary)
            Text(preview.summary)
                .font(.title3)
                .fixedSize(horizontal: false, vertical: true)
            ForEach(Array(preview.detail.enumerated()), id: \.offset) { _, line in
                Text(line)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color(nsColor: .controlBackgroundColor))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.08))
        )
        .padding(.horizontal, 20)
        .padding(.top, 18)
    }

    @ViewBuilder
    private func paneBody(_ pane: SettingsPane) -> some View {
        switch pane.id {
        case .prices:
            placeholder("The editable price table arrives in the next commit.")
        case .advanced:
            placeholder("Paths, index statistics, reindex, reset and About arrive in the next commit.")
        case .diagnostics:
            // Already written, already correct — moved from the old tab bar
            // into the split view rather than left dark for a commit.
            DiagnosticsView(engine: engine, refreshToken: refreshToken)
        default:
            SchemaPane(pane: pane, actions: actions)
        }
    }

    private func placeholder(_ text: String) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
            .padding(20)
    }

    private var emptyDetail: some View {
        VStack(spacing: 8) {
            Text("No setting matches “\(query)”.")
                .font(.title3)
            Text("Search looks at every label and every explanation, so try a word you would expect to read here.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// The three things macOS can say about notifications that the stored
    /// setting cannot — kept verbatim from the window this one replaces,
    /// each of them a defect someone found once already.
    @ViewBuilder
    private var notificationPermissionNotes: some View {
        VStack(alignment: .leading, spacing: 6) {
            if notifier.authorization == .denied {
                Text("Perch isn't allowed to show notifications.")
                    .foregroundStyle(.red)
                Button("Open Notification Settings…") { openSystemNotificationSettings() }
                    .buttonStyle(.link)
            } else if !notifier.canDeliver && storedWaitingEnabled {
                // The settings file says on, macOS has never been asked. The
                // toggle below already reads OFF, which is the honest state —
                // this says why it disagrees with the file the user edited,
                // and what to do about it.
                Text("The settings file turns these on, but macOS hasn't been asked for permission yet. Switch this on here to ask.")
                    .foregroundStyle(.orange)
            }
            // macOS refused a delivery Rust had already decided on. Reported
            // verbatim rather than swallowed — see `Notifier.lastFailure`.
            if let failure = notifier.lastFailure {
                Text(failure).foregroundStyle(.red)
            }
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.top, 12)
    }

    private func banner(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(color)
            .fixedSize(horizontal: false, vertical: true)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(color.opacity(0.12))
    }

    /// The single place a semantic icon becomes a macOS glyph. Rust names the
    /// concept; only this function knows what SF Symbols calls it. Monochrome
    /// by choice: nine panes of one app do not need colour to be told apart.
    private func symbolName(_ icon: IconId) -> String {
        switch icon {
        case .general:       return "gearshape"
        case .menuBar:       return "menubar.rectangle"
        case .popover:       return "rectangle.on.rectangle"
        case .projects:      return "folder"
        case .usage:         return "chart.bar"
        case .prices:        return "dollarsign.circle"
        case .notifications: return "bell"
        case .diagnostics:   return "stethoscope"
        case .advanced:      return "slider.horizontal.3"
        }
    }

    private func openSystemNotificationSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.notifications") else { return }
        NSWorkspace.shared.open(url)
    }

    // MARK: - Selection

    /// A filtered sidebar can drop the remembered pane; the detail side then
    /// falls back to whatever the search did return, *without* overwriting
    /// what the user last deliberately chose.
    private var currentPane: SettingsPane? {
        panes.first { $0.id.key == selectedPaneKey } ?? panes.first
    }

    private var selection: Binding<String?> {
        Binding(
            get: { currentPane?.id.key },
            set: { key in if let key { selectedPaneKey = key } }
        )
    }

    // MARK: - Actions handed to the renderer

    private var actions: SchemaPaneActions {
        SchemaPaneActions(
            write: { key, value in write(key, value) },
            reset: { pane in reset(pane) },
            perform: { _ in
                // No schema pane carries an action row yet; the buttons that
                // do anything (reindex, reset everything) belong to the
                // bespoke Advanced pane.
            },
            additionallyDisabled: { row in
                guard let key = row.key else { return false }
                switch key {
                case .waitingAfterMinutes, .includeBackground, .sound:
                    return !notifier.canDeliver
                default:
                    return false
                }
            },
            systemToggle: { key in systemToggle(key) }
        )
    }

    /// The two toggles whose displayed state is macOS's answer rather than
    /// the file's, because a setting that lies about its own effect is worse
    /// than no setting at all.
    private func systemToggle(_ key: SettingKey) -> SystemToggle? {
        switch key {
        case .launchAtLogin:
            // Reads the actual registration, never the stored value: a user
            // can remove the login item from System Settings without Perch
            // hearing about it. `set` registers first and only persists on
            // success, so a refused request never leaves a stored `true`
            // with nothing behind it.
            return SystemToggle(
                isOn: LoginItem.isRegistered,
                isDisabled: false,
                set: { on in
                    do {
                        try LoginItem.setRegistered(on)
                        write(.launchAtLogin, .bool(value: on))
                    } catch {
                        actionError = error.localizedDescription
                    }
                }
            )
        case .waitingEnabled:
            // Never ON over a channel delivering nothing — a revoked
            // permission, and equally a `waiting_enabled = true` hand-edited
            // into config.toml that macOS has never been asked about. The ON
            // branch asks for authorization at the moment the user asks for
            // it, and persists only what was granted.
            return SystemToggle(
                isOn: notifier.canDeliver && storedWaitingEnabled,
                isDisabled: notifier.authorization == .denied,
                set: { on in
                    guard on else {
                        write(.waitingEnabled, .bool(value: false))
                        return
                    }
                    Task {
                        let granted = await engine.notifier.requestAuthorization()
                        write(.waitingEnabled, .bool(value: granted))
                    }
                }
            )
        default:
            return nil
        }
    }

    /// What the settings file says, before macOS gets a say — read out of the
    /// schema rather than kept in a second place.
    private var storedWaitingEnabled: Bool {
        guard let row = row(.waitingEnabled), case let .toggle(on) = row.control else { return false }
        return on
    }

    private func row(_ key: SettingKey) -> SettingRow? {
        panes.lazy.flatMap { $0.groups }.flatMap { $0.rows }.first { $0.key == key }
    }

    // MARK: - Load, search, write

    /// Runs once per `refreshToken` — i.e. once per fresh presentation of the
    /// window, including a reopen after it was previously closed.
    /// `SettingsWindowController.show()` reuses the same `NSHostingView` and
    /// only swaps `.rootView`, so SwiftUI preserves this view's `@State`
    /// across a close/reopen (the same reuse that makes `refreshToken`
    /// necessary at all) — without clearing `actionError` here, a write that
    /// failed, closed, and was reopened later would show a stale failure
    /// banner with nothing currently wrong. Cleared unconditionally at the
    /// top, before the fetch below, rather than only on success: clearing it
    /// afterwards would leave a stale `actionError` alongside a fresh
    /// `loadError` if the engine happened to be unavailable on this reopen.
    private func load() async {
        actionError = nil
        notes = []
        guard let fetched = await engine.settingsSchema(query: queryOrNil) else {
            loadError = EngineUnavailable().localizedDescription
            return
        }
        panes = fetched
        loadError = nil
        // Never prompts — just reads whatever the OS currently says, so a
        // permission granted or denied from System Settings since this window
        // last loaded is reflected the next time it opens.
        await engine.notifier.refreshAuthorization()
    }

    /// The query goes to Rust. Swift does no matching of its own; whatever
    /// comes back *is* the sidebar.
    private func search(_ q: String) {
        searchTask?.cancel()
        searchTask = Task {
            let fetched = await engine.settingsSchema(query: q.isEmpty ? nil : q)
            guard !Task.isCancelled else { return }
            guard let fetched else {
                loadError = EngineUnavailable().localizedDescription
                return
            }
            panes = fetched
            loadError = nil
        }
    }

    private var queryOrNil: String? { query.isEmpty ? nil : query }

    private func write(_ key: SettingKey, _ value: SettingValue) {
        enqueue {
            switch await engine.setSetting(key: key, value: value) {
            case .success(let result):
                await adopt(result)
            case .failure(let error):
                // Must reach the user independent of `loadError` — see
                // `actionError`'s declaration above.
                actionError = error.localizedDescription
            }
        }
    }

    private func reset(_ pane: PaneId) {
        enqueue {
            switch await engine.resetPane(pane) {
            case .success(let result):
                await adopt(result)
            case .failure(let error):
                actionError = error.localizedDescription
            }
        }
    }

    /// Queues `work` behind whatever write is already pending, rather than
    /// firing it against the schema as it stands right now. Without this,
    /// two controls edited in quick succession (ordinary use, not a stress
    /// case) would both read the same pre-edit schema if the first round
    /// trip hadn't returned — a plain read-modify-write race in which
    /// whichever response lands last silently discards the other edit. Each
    /// write names one key rather than a whole struct, so the two edits no
    /// longer overwrite each other's *fields*; they still race over which
    /// schema the second control reads its `get` from, and over which
    /// `notes` the user is left looking at.
    private func enqueue(_ work: @escaping () async -> Void) {
        let previous = pendingSave
        pendingSave = Task {
            _ = await previous?.value
            actionError = nil
            await work()
        }
    }

    /// A write returns the whole schema plus whatever `validated` changed, so
    /// nothing here re-reads to see its own write. While a search is on,
    /// though, the returned panes are the *unfiltered* ones — adopting them
    /// as they stand would silently drop the filter out from under the
    /// sidebar, so the filtered view is asked for again.
    private func adopt(_ result: SettingsResult) async {
        notes = result.notes
        guard let q = queryOrNil else {
            panes = result.panes
            return
        }
        panes = await engine.settingsSchema(query: q) ?? result.panes
    }
}

private extension PaneId {
    /// A stable string for `@AppStorage` and for list selection. Never shown
    /// to the user — every visible pane name is `SettingsPane.title`, which
    /// Rust writes.
    var key: String {
        switch self {
        case .general:       return "general"
        case .menuBar:       return "menuBar"
        case .popover:       return "popover"
        case .projects:      return "projects"
        case .usage:         return "usage"
        case .prices:        return "prices"
        case .notifications: return "notifications"
        case .diagnostics:   return "diagnostics"
        case .advanced:      return "advanced"
        }
    }
}

/// Remembers how wide the user dragged the sidebar. SwiftUI's
/// `NavigationSplitView` takes an *ideal* width and hands back no binding for
/// the width that results, so the only honest way to persist a dragged column
/// is to measure the one being drawn and feed it back as next time's ideal.
private struct SidebarWidthReporter: View {
    @Binding var width: Double

    var body: some View {
        GeometryReader { proxy in
            Color.clear
                .onChange(of: proxy.size.width) { _, w in
                    guard w > 1 else { return }
                    width = w
                }
        }
    }
}
