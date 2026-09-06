import AppKit
import SwiftUI
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
/// on every keystroke.
struct SettingsRootView: View {
    let engine: PerchEngine
    let refreshToken: Int

    @State private var model: SettingsModel?
    @State private var loadError: String?

    @State private var claudeDirDraft: String = ""
    @FocusState private var claudeDirFocused: Bool

    enum Tab: Hashable { case general, sessions, notifications, diagnostics }
    @State private var tab: Tab = .general

    var body: some View {
        VStack(spacing: 0) {
            if let model {
                // Both must reach the user: `error` means a hand-edited file
                // failed to parse at all (running on defaults); `notes` means
                // it parsed but one or more values were out of range and got
                // clamped. Neither is silent.
                if let error = model.error {
                    banner(error, color: .red)
                }
                if !model.notes.isEmpty {
                    banner(model.notes.joined(separator: "\n"), color: .orange)
                }
            } else if let loadError {
                banner(loadError, color: .red)
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
                DiagnosticsView(engine: engine)
                    .tabItem { Label("Diagnostics", systemImage: "stethoscope") }
                    .tag(Tab.diagnostics)
            }
        }
        .frame(width: 520, height: 420)
        .task(id: refreshToken) { await load() }
        .onChange(of: claudeDirFocused) { _, focused in
            if !focused { Task { await commitClaudeDir() } }
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
                    .onSubmit { Task { await commitClaudeDir() } }
                HStack {
                    Button("Choose…") { chooseClaudeDir() }
                    if !claudeDirDraft.isEmpty {
                        Button("Use Default (Auto-Detect)") {
                            claudeDirDraft = ""
                            Task { await commitClaudeDir() }
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
        Task { await commitClaudeDir() }
    }

    private func commitClaudeDir() async {
        guard let model, claudeDirDraft != model.settings.claudeConfigDir else { return }
        await update { $0.claudeConfigDir = claudeDirDraft }
    }

    // MARK: - Sessions & Menu Bar

    @ViewBuilder
    private var sessionsPane: some View {
        Form {
            Stepper(value: pollSecondsBinding, in: 1...60) {
                Text("Check every \(model?.settings.pollSeconds ?? 5) seconds")
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

            // Dependent controls stay visible and greyed rather than
            // disappearing — hiding them would conceal what is configurable
            // and make the window twitch when the toggle above flips.
            let enabled = model?.settings.waitingEnabled ?? false

            Stepper(value: waitingAfterMinutesBinding, in: 1...240) {
                Text("After \(model?.settings.waitingAfterMinutes ?? 10) minutes")
            }
            .disabled(!enabled)

            Toggle("Include background sessions", isOn: includeBackgroundBinding)
                .disabled(!enabled)
        }
        .padding(20)
        .disabled(model == nil)
    }

    // MARK: - Bindings

    /// Every binding below saves immediately: each is a discrete, deliberate
    /// edit (a flip, a single-choice pick, a stepper click), not free typing,
    /// so there is no keystroke-storm risk in writing it straight through.

    private var launchAtLoginBinding: Binding<Bool> {
        Binding(
            get: { model?.settings.launchAtLogin ?? false },
            set: { v in Task { await update { $0.launchAtLogin = v } } }
        )
    }

    private var preferredTerminalBinding: Binding<String> {
        Binding(
            get: { model?.settings.preferredTerminal ?? "Terminal" },
            set: { v in Task { await update { $0.preferredTerminal = v } } }
        )
    }

    private var pollSecondsBinding: Binding<UInt32> {
        Binding(
            get: { model?.settings.pollSeconds ?? 5 },
            set: { v in Task { await update { $0.pollSeconds = v } } }
        )
    }

    private var menuBarDisplayBinding: Binding<MenuBarDisplay> {
        Binding(
            get: { model?.settings.menuBarDisplay ?? .count },
            set: { v in Task { await update { $0.menuBarDisplay = v } } }
        )
    }

    private var waitingEnabledBinding: Binding<Bool> {
        Binding(
            get: { model?.settings.waitingEnabled ?? false },
            set: { v in Task { await update { $0.waitingEnabled = v } } }
        )
    }

    private var waitingAfterMinutesBinding: Binding<UInt32> {
        Binding(
            get: { model?.settings.waitingAfterMinutes ?? 10 },
            set: { v in Task { await update { $0.waitingAfterMinutes = v } } }
        )
    }

    private var includeBackgroundBinding: Binding<Bool> {
        Binding(
            get: { model?.settings.includeBackground ?? false },
            set: { v in Task { await update { $0.includeBackground = v } } }
        )
    }

    // MARK: - Load / save

    private func load() async {
        guard let m = await engine.settings() else {
            loadError = EngineUnavailable().localizedDescription
            return
        }
        apply(m)
    }

    private func update(_ mutate: (inout PerchFFI.Settings) -> Void) async {
        guard var settings = model?.settings else { return }
        mutate(&settings)
        switch await engine.saveSettings(settings) {
        case .success(let m):
            apply(m)
        case .failure(let e):
            loadError = e.localizedDescription
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
