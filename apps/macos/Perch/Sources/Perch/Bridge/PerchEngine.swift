import Foundation
import PerchFFI

/// Owns the Rust `Perch` object and republishes its model on the main actor.
/// Nothing in Swift reads Claude Code's files; this is the only doorway.
@MainActor
final class PerchEngine: ObservableObject {
    @Published private(set) var model: PopoverModel?
    @Published private(set) var startupError: String?

    /// Owns delivery of the "waiting on you" banner — see its own doc
    /// comment. Created eagerly, not lazily, so its
    /// `UNUserNotificationCenterDelegate` is registered before the engine
    /// itself even starts, in case the app is being launched by clicking a
    /// notification left over from a previous run.
    let notifier = Notifier()

    private var perch: Perch?
    private var listener: Listener?

    func start() {
        do {
            let p = try Perch(configDir: nil)
            let l = Listener(
                deliverModel: { [weak self] m in
                    Task { @MainActor in self?.model = m }
                },
                deliverNotifications: { [weak self] items in
                    Task { @MainActor in await self?.notifier.deliver(items) }
                }
            )
            perch = p
            listener = l
            model = p.current()
            p.start(listener: l)
        } catch {
            // Rust's `PerchError` already carries a finished, user-facing
            // message via `LocalizedError` (UniFFI exposes it); `String(describing:)`
            // would instead dump the Swift enum case, e.g. `NoConfigDir(path: "...")`.
            startupError = error.localizedDescription
        }
    }

    func refresh() { perch?.refresh() }
    func stop() { perch?.stop() }

    /// Reads run off the main thread: a large index makes these slow, and this
    /// is called while AppKit is preparing to show a window.
    func mainWindow() async -> MainWindowModel? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.mainWindow() }.value
    }

    func projectDetail(_ id: Int64) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.projectDetail(projectId: id) }
        }.value
    }

    func usage() async -> Result<UsageModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.usage() }
        }.value
    }

    /// Every edit below returns the refreshed `ProjectDetail` from Rust, so
    /// the caller adopts it directly instead of re-fetching or guessing.

    func setNote(projectId: Int64, note: String) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setNote(projectId: projectId, note: note) }
        }.value
    }

    func setPinned(projectId: Int64, pinned: Bool) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setPinned(projectId: projectId, pinned: pinned) }
        }.value
    }

    func setArchived(projectId: Int64, archived: Bool) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setArchived(projectId: projectId, archived: archived) }
        }.value
    }

    func renameProject(projectId: Int64, name: String) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.renameProject(projectId: projectId, name: name) }
        }.value
    }

    func setNotifyOverride(projectId: Int64, override: NotifyOverride) async -> Result<ProjectDetail, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setNotifyOverride(projectId: projectId, o: override) }
        }.value
    }

    /// One session's own menu, for the submenu hanging off its popover row.
    ///
    /// Synchronous, unlike every other read here, because AppKit asks for a
    /// submenu's contents from `menuNeedsUpdate(_:)` and cannot await. The
    /// work is one directory read and a couple of indexed lookups — the same
    /// shape of read `current()` already performs on the main thread at
    /// startup.
    ///
    /// `owningApp` is the application the shell resolved the session's pid
    /// to: Rust phrases the Focus action, but only AppKit can say what is
    /// running. `nil` back means the session is no longer live (or the engine
    /// never started).
    func sessionMenu(sessionId: String, owningApp: String?) -> SessionMenu? {
        perch?.sessionMenu(sessionId: sessionId, owningApp: owningApp)
    }

    /// Builders are pure (non-throwing) on the Rust side; `nil` here only
    /// means the engine itself isn't running.
    func resumeCommand(sessionId: String, cwd: String) async -> TerminalCommand? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) {
            perch.resumeCommand(sessionId: sessionId, cwd: cwd)
        }.value
    }

    func openCommand(cwd: String) async -> TerminalCommand? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) {
            perch.openCommand(cwd: cwd)
        }.value
    }

    /// Which terminal Resume and Open should launch, read at the moment of
    /// launch rather than cached — so a change made in the settings window
    /// (or hand-edited into `config.toml`) takes effect on the very next
    /// launch. `nil` only means the engine itself isn't running.
    func preferredTerminal() async -> String? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.preferredTerminal() }.value
    }

    // MARK: - The settings schema
    //
    // The whole settings window is built on these: Rust owns the panes, the
    // captions, the enablement and the search, and every write names one key
    // rather than shipping a whole struct back. `nil` from `settingsSchema`
    // means the engine itself isn't running — Rust's own read never fails.

    /// `query` is matched in Rust by `filter_panes`. Swift sends keystrokes
    /// and draws whatever comes back; it does no matching of its own, or the
    /// window would carry a second, staler copy of every label.
    /// Carries `notes` as well as the panes: a value clamped on the way *in*
    /// (a `poll_seconds = 9999` hand-edited into `config.toml`) is reported
    /// by the read that found it, not only by some later write.
    func settingsSchema(query: String?) async -> SettingsResult? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) {
            perch.settingsSchema(query: query)
        }.value
    }

    /// Store one value under one key. Returns the schema that resulted plus
    /// whatever `validated` had to change on the way to disk, so the caller
    /// never re-reads to see its own write and never has to discover a clamp
    /// for itself.
    func setSetting(key: SettingKey, value: SettingValue) async -> Result<SettingsResult, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setSetting(key: key, value: value) }
        }.value
    }

    /// Restore every setting one pane shows to its factory value, leaving
    /// every other pane alone.
    func resetPane(_ pane: PaneId) async -> Result<SettingsResult, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.resetPane(pane: pane) }
        }.value
    }

    /// Restore every setting at once. One call, not twenty-seven: a reset that
    /// failed halfway would leave a configuration that is neither what Perch
    /// ships nor what the user chose.
    func resetAllSettings() async -> Result<SettingsResult, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.resetAllSettings() }
        }.value
    }

    // MARK: - Prices
    //
    // Every one of these returns the whole pane — the table *and* the models
    // in use that still have no rate — so the caller adopts one value and
    // the two can never disagree about what is missing. Nothing here mutates
    // anything locally first: a control that moved before its write landed
    // has nothing to move back to when the write fails.

    func prices() async -> Result<PricesModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.prices() }
        }.value
    }

    /// The rates travel as the text the user typed. Rust parses them, and a
    /// field it cannot read is refused with a sentence naming the column —
    /// nothing is stored, so the rate already in force stays in force.
    func setPrice(model: String, rates: RateFields) async -> Result<PricesModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.setPrice(model: model, rates: rates) }
        }.value
    }

    func removePrice(model: String) async -> Result<PricesModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.removePrice(model: model) }
        }.value
    }

    func resetPrices() async -> Result<PricesModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.resetPrices() }
        }.value
    }

    // MARK: - Advanced

    func advanced() async -> AdvancedModel? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.advanced() }.value
    }

    /// Rebuilds the index from Claude Code's session files. Slow by nature —
    /// it reads every one of them — and it returns the pane the run
    /// produced, so the new counts are this run's rather than the old ones
    /// redrawn.
    func reindexNow() async -> Result<AdvancedModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.reindexNow() }
        }.value
    }

    /// Backs the diagnostics pane — a full re-index can make this slow, so
    /// it gets the same off-main-thread treatment as every other read here.
    func diagnostics() async -> DiagnosticsModel? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.diagnostics() }.value
    }
}

struct EngineUnavailable: LocalizedError {
    var errorDescription: String? { "Perch's engine is not running." }
}

/// Rust calls both methods on its watcher thread; hop to the main actor before touching UI.
private final class Listener: PerchListener, @unchecked Sendable {
    private let deliverModel: @Sendable (PopoverModel) -> Void
    private let deliverNotifications: @Sendable ([WaitingNotification]) -> Void

    init(
        deliverModel: @escaping @Sendable (PopoverModel) -> Void,
        deliverNotifications: @escaping @Sendable ([WaitingNotification]) -> Void
    ) {
        self.deliverModel = deliverModel
        self.deliverNotifications = deliverNotifications
    }

    func onModel(model: PopoverModel) { deliverModel(model) }
    func onNotifications(items: [WaitingNotification]) { deliverNotifications(items) }
}
