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
    var isWindowOpen: Bool { window?.isVisible ?? false }

    func show() {
        refreshToken += 1

        if let window {
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
            if window.isMiniaturized {
                window.deminiaturize(nil)
            }
            window.makeKeyAndOrderFront(nil)
            (window.contentView as? NSHostingView<SettingsRootView>)?.rootView =
                SettingsRootView(engine: engine, refreshToken: refreshToken)
            return
        }

        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 420),
            styleMask: [.titled, .closable, .miniaturizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        w.title = "Perch Settings"
        w.center()
        w.isReleasedWhenClosed = false
        w.delegate = self
        w.contentView = NSHostingView(rootView: SettingsRootView(engine: engine, refreshToken: refreshToken))
        window = w

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
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

/// The settings window's content: three editable panes plus Diagnostics,
/// reachable directly rather than hidden behind a toggle.
///
/// Per-project overrides (notifications, pin, archive, note) live on the
/// project in the main window's detail pane, not here — this view only ever
/// edits the eight global `Settings` fields. It deliberately does not list
/// projects.
///
/// **Save timing:** every discrete control (toggle, picker, stepper) saves
/// immediately on change, adopting the returned `SettingsModel` exactly like
/// `ProjectDetailPane`'s pin/archive toggles do. The one free-text field —
/// the Claude Code directory path — saves on commit (focus loss, Return, or
/// the folder picker) instead, mirroring that same file's note editor: a
/// path typed character by character would otherwise rewrite the config file
/// on every keystroke. Saves themselves are queued one at a time (see
/// `update(_:)`/`pendingSave`) so two controls edited in quick succession
/// never race each other over the same whole-struct `saveSettings` call.
struct SettingsRootView: View {
    let engine: PerchEngine
    let refreshToken: Int

    /// Observed, not merely read: `Notifier` re-reads macOS's authorization
    /// on every delivery, so a permission granted or revoked while this
    /// window is open — or a delivery macOS refused — repaints the pane
    /// below without waiting for a reopen.
    @ObservedObject private var notifier: Notifier

    @MainActor
    init(engine: PerchEngine, refreshToken: Int) {
        self.engine = engine
        self.refreshToken = refreshToken
        _notifier = ObservedObject(wrappedValue: engine.notifier)
    }

    @State private var model: SettingsModel?
    /// Set only when the *initial* `settings()` read fails (the engine isn't
    /// running) — rendered only while there is no model yet.
    @State private var loadError: String?
    /// Set when a `saveSettings` call fails, independent of `loadError`.
    /// Rendered unconditionally, whether or not a model has loaded — mirrors
    /// `ProjectDetailPane.actionError`, which exists for exactly this reason:
    /// a post-load action failure must never be gated behind the "still
    /// loading" branch, or it silently disappears once a model exists.
    @State private var actionError: String?

    @State private var claudeDirDraft: String = ""
    @FocusState private var claudeDirFocused: Bool

    /// Chains queued saves so a second control's edit always builds on the
    /// model left by the first save's `apply()`, not on the pre-edit
    /// baseline both would otherwise read if their `saveSettings` calls
    /// overlapped — see `update(_:)`.
    @State private var pendingSave: Task<Void, Never>?

    enum Tab: Hashable { case general, sessions, notifications, diagnostics }
    @State private var tab: Tab = .general

    /// Rust's own `Settings::default()`, read once. Every control below falls
    /// back to these while `model` is still nil, rather than to a Swift
    /// literal of the same value: a literal here is a second source of truth
    /// for a default Rust already owns, and it drifts silently the day that
    /// one changes. `preferredTerminal` is the one field still defaulted by a
    /// literal below — the whole terminal-choice seam is being replaced next
    /// milestone, when Rust supplies the detected-terminal list.
    private static let defaults = PerchFFI.defaultSettings()

    /// macOS has been asked and said no: the toggle is forced off *and*
    /// disabled, because nothing the user does in this window can change it.
    private var notificationsDenied: Bool { notifier.authorization == .denied }

    /// Anything short of "would actually be shown" — `.notDetermined`
    /// included. This, not `notificationsDenied`, is what the toggle and its
    /// dependent controls read: `.notDetermined` is the state a
    /// `waiting_enabled = true` hand-edited into config.toml leaves behind,
    /// and it delivers exactly as little as a denial. The toggle stays
    /// *enabled* in that state, though — flipping it on is precisely what
    /// asks macOS for permission.
    private var notificationsDeliverable: Bool { notifier.canDeliver }

    var body: some View {
        VStack(spacing: 0) {
            if let loadError, model == nil {
                banner(loadError, color: .red)
            }
            if let actionError {
                banner(actionError, color: .red)
            }
            if let model {
                // Both must reach the user: `error` means a hand-edited file
                // failed to parse at all (running on defaults); `notes` means
                // it parsed but Rust refused a value as it stood — a number
                // clamped to its range, or a `claude_config_dir` that is not
                // a directory, dropped back to auto-detection. Neither is
                // silent, and `notes` is how a rejected directory reaches the
                // user at save time rather than at the next launch.
                if let error = model.error {
                    banner(error, color: .red)
                }
                if !model.notes.isEmpty {
                    banner(model.notes.joined(separator: "\n"), color: .orange)
                }
            }

            TabView(selection: $tab) {
                generalPane
                    .tabItem { Label("General", systemImage: "gearshape") }
                    .tag(Tab.general)
                sessionsPane
                    .tabItem { Label("Sessions & Menu Bar", systemImage: "clock") }
                    .tag(Tab.sessions)
                notificationsPane
                    .tabItem { Label("Notifications", systemImage: "bell") }
                    .tag(Tab.notifications)
                DiagnosticsView(engine: engine, refreshToken: refreshToken)
                    .tabItem { Label("Diagnostics", systemImage: "stethoscope") }
                    .tag(Tab.diagnostics)
            }
        }
        .frame(width: 520, height: 420)
        .task(id: refreshToken) { await load() }
        .onChange(of: claudeDirFocused) { _, focused in
            if !focused { commitClaudeDir() }
        }
    }

    private func banner(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(color)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(color.opacity(0.12))
    }

    // MARK: - General

    @ViewBuilder
    private var generalPane: some View {
        Form {
            Toggle("Launch at login", isOn: launchAtLoginBinding)

            Section("Claude Code Directory") {
                TextField("Auto-detected", text: $claudeDirDraft)
                    .focused($claudeDirFocused)
                    .onSubmit { commitClaudeDir() }
                HStack {
                    Button("Choose…") { chooseClaudeDir() }
                    if !claudeDirDraft.isEmpty {
                        Button("Use Default (Auto-Detect)") {
                            claudeDirDraft = ""
                            commitClaudeDir()
                        }
                    }
                }
            }

            Section("Preferred Terminal") {
                Picker("Preferred terminal", selection: preferredTerminalBinding) {
                    Text("Terminal").tag("Terminal")
                    Text("iTerm2").tag("iTerm2")
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }
        }
        .padding(20)
        .disabled(model == nil)
    }

    private func chooseClaudeDir() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        if !claudeDirDraft.isEmpty {
            panel.directoryURL = URL(fileURLWithPath: claudeDirDraft)
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        claudeDirDraft = url.path
        commitClaudeDir()
    }

    private func commitClaudeDir() {
        guard let model, claudeDirDraft != model.settings.claudeConfigDir else { return }
        update { $0.claudeConfigDir = claudeDirDraft }
    }

    // MARK: - Sessions & Menu Bar

    @ViewBuilder
    private var sessionsPane: some View {
        Form {
            Stepper(value: pollSecondsBinding, in: 1...60) {
                Text(pollSecondsLabel(seconds: pollSecondsBinding.wrappedValue))
            }

            Picker("Menu bar shows", selection: menuBarDisplayBinding) {
                Text("Icon only").tag(MenuBarDisplay.icon)
                Text("Icon + count").tag(MenuBarDisplay.count)
                Text("Icon + count + waiting").tag(MenuBarDisplay.countAndWaiting)
            }
        }
        .padding(20)
        .disabled(model == nil)
    }

    // MARK: - Notifications

    @ViewBuilder
    private var notificationsPane: some View {
        Form {
            Toggle("Notify when a session is waiting on you", isOn: waitingEnabledBinding)
                .disabled(notificationsDenied)

            if notificationsDenied {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Perch isn't allowed to show notifications.")
                        .foregroundStyle(.red)
                    Button("Open Notification Settings…") { openSystemNotificationSettings() }
                }
                .font(.callout)
                .padding(.vertical, 4)
            } else if !notificationsDeliverable && storedWaitingEnabled {
                // The settings file says on, macOS has never been asked. The
                // toggle above already reads OFF, which is the honest state —
                // this says why it disagrees with the file the user edited,
                // and what to do about it.
                Text("The settings file turns these on, but macOS hasn't been asked for permission yet. Switch this on here to ask.")
                    .foregroundStyle(.orange)
                    .font(.callout)
                    .padding(.vertical, 4)
            }

            // macOS refused a delivery Rust had already decided on. Reported
            // verbatim rather than swallowed — see `Notifier.lastFailure`.
            if let failure = notifier.lastFailure {
                Text(failure)
                    .foregroundStyle(.red)
                    .font(.callout)
                    .padding(.vertical, 4)
            }

            // Dependent controls stay visible and greyed rather than
            // disappearing — hiding them would conceal what is configurable
            // and make the window twitch when the toggle above flips.
            let enabled = storedWaitingEnabled && notificationsDeliverable

            Stepper(value: waitingAfterMinutesBinding, in: 1...240) {
                Text(waitingAfterMinutesLabel(minutes: waitingAfterMinutesBinding.wrappedValue))
            }
            .disabled(!enabled)

            Toggle("Include background sessions", isOn: includeBackgroundBinding)
                .disabled(!enabled)
        }
        .padding(20)
        .disabled(model == nil)
    }

    /// Deep-links into System Settings' Notifications pane — the same
    /// `x-apple.systempreferences:` scheme every third-party Mac app uses for
    /// this, since there is no public API to open a single app's own
    /// notification settings entry directly.
    private func openSystemNotificationSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.notifications") else { return }
        NSWorkspace.shared.open(url)
    }

    // MARK: - Bindings

    /// Every binding below saves immediately: each is a discrete, deliberate
    /// edit (a flip, a single-choice pick, a stepper click), not free typing,
    /// so there is no keystroke-storm risk in writing it straight through.

    /// `get` reads macOS's own registration (`LoginItem.isRegistered`), never
    /// the stored `launchAtLogin` value — the two can disagree (removed via
    /// System Settings, or a fresh install with a stale hand-edited file),
    /// and the control must always reflect what is actually registered, not
    /// what Perch last wrote to disk. `set` registers/unregisters first;
    /// only on success does it persist the setting, so a refused request
    /// never leaves a stored `true` that isn't backed by a real
    /// registration.
    private var launchAtLoginBinding: Binding<Bool> {
        Binding(
            get: { LoginItem.isRegistered },
            set: { v in
                do {
                    try LoginItem.setRegistered(v)
                    update { $0.launchAtLogin = v }
                } catch {
                    actionError = error.localizedDescription
                }
            }
        )
    }

    private var preferredTerminalBinding: Binding<String> {
        Binding(
            get: { model?.settings.preferredTerminal ?? "Terminal" },
            set: { v in update { $0.preferredTerminal = v } }
        )
    }

    private var pollSecondsBinding: Binding<UInt32> {
        Binding(
            get: { model?.settings.pollSeconds ?? Self.defaults.pollSeconds },
            set: { v in update { $0.pollSeconds = v } }
        )
    }

    private var menuBarDisplayBinding: Binding<MenuBarDisplay> {
        Binding(
            get: { model?.settings.menuBarDisplay ?? Self.defaults.menuBarDisplay },
            set: { v in update { $0.menuBarDisplay = v } }
        )
    }

    /// What the settings file says, before macOS gets a say.
    private var storedWaitingEnabled: Bool {
        model?.settings.waitingEnabled ?? Self.defaults.waitingEnabled
    }

    /// `get` reports `false` unless a notification would actually be shown —
    /// a revoked permission, and equally a `waiting_enabled = true` hand-
    /// edited into config.toml that macOS has never been asked about, so the
    /// toggle can never sit on over a channel delivering nothing. `set`'s ON
    /// branch requests authorization *at the moment the user asks for it*,
    /// never before: granted, it saves `true`; refused, it leaves the
    /// persisted setting `false` rather than saving a preference that would
    /// silently do nothing. Both branches leave `notifier.authorization`
    /// holding whatever macOS now says — `requestAuthorization` re-reads it —
    /// so this binding's `get` needs no separate flag of its own to keep in
    /// step.
    private var waitingEnabledBinding: Binding<Bool> {
        Binding(
            get: { notificationsDeliverable && storedWaitingEnabled },
            set: { v in
                guard v else {
                    update { $0.waitingEnabled = false }
                    return
                }
                Task {
                    let granted = await engine.notifier.requestAuthorization()
                    update { $0.waitingEnabled = granted }
                }
            }
        )
    }

    private var waitingAfterMinutesBinding: Binding<UInt32> {
        Binding(
            get: { model?.settings.waitingAfterMinutes ?? Self.defaults.waitingAfterMinutes },
            set: { v in update { $0.waitingAfterMinutes = v } }
        )
    }

    private var includeBackgroundBinding: Binding<Bool> {
        Binding(
            get: { model?.settings.includeBackground ?? Self.defaults.includeBackground },
            set: { v in update { $0.includeBackground = v } }
        )
    }

    // MARK: - Load / save

    /// Runs once per `refreshToken` — i.e. once per fresh presentation of the
    /// window, including a reopen after it was previously closed.
    /// `SettingsWindowController.show()` reuses the same `NSHostingView` and
    /// only swaps `.rootView`, so SwiftUI preserves this view's `@State`
    /// across a close/reopen (the same reuse that makes `refreshToken`
    /// necessary at all) — without clearing `actionError` here, a save that
    /// failed, closed, and was reopened later would show a stale
    /// save-failure banner with nothing currently wrong. Cleared
    /// unconditionally at the top, before the fetch below, rather than only
    /// in `apply()`: `apply()` only runs if `engine.settings()` succeeds, so
    /// clearing there would leave a stale `actionError` on screen alongside
    /// a fresh `loadError` if the engine happened to be unavailable on this
    /// particular reopen. This does not affect an in-session save: nothing
    /// but a reopen re-runs `load()`, so a banner a save just produced is
    /// never wiped before the user can read it.
    private func load() async {
        actionError = nil
        guard let m = await engine.settings() else {
            loadError = EngineUnavailable().localizedDescription
            return
        }
        apply(m)
        // Never prompts — just reads whatever the OS currently says, so a
        // permission granted or denied from System Settings since this window
        // last loaded (including from a run that never touched this toggle at
        // all) is reflected the next time the window opens. Every subsequent
        // delivery re-reads it too, so an open window keeps up.
        await engine.notifier.refreshAuthorization()
    }

    /// Queues `mutate` behind whatever save is already pending, rather than
    /// firing it immediately against `model` as it stands right now. Without
    /// this, two controls edited in quick succession (ordinary use, not a
    /// stress case) would both read the same pre-edit `model.settings` if
    /// the first `saveSettings` round trip hadn't returned yet — a plain
    /// read-modify-write race in which whichever response lands last
    /// silently discards the other edit. Chaining through `pendingSave`
    /// instead means the second `mutate` only runs after the first save's
    /// `apply()` has already landed, so it always builds on the freshest
    /// `model`, not a stale baseline both edits started from.
    private func update(_ mutate: @escaping (inout PerchFFI.Settings) -> Void) {
        let previous = pendingSave
        pendingSave = Task {
            _ = await previous?.value
            await performUpdate(mutate)
        }
    }

    private func performUpdate(_ mutate: (inout PerchFFI.Settings) -> Void) async {
        guard var settings = model?.settings else { return }
        mutate(&settings)
        actionError = nil
        switch await engine.saveSettings(settings) {
        case .success(let m):
            apply(m)
        case .failure(let e):
            // Must reach the user independent of `model`/`loadError` — see
            // `actionError`'s declaration above.
            actionError = e.localizedDescription
        }
    }

    private func apply(_ m: SettingsModel) {
        model = m
        loadError = nil
        if !claudeDirFocused {
            claudeDirDraft = m.settings.claudeConfigDir
        }
    }
}
