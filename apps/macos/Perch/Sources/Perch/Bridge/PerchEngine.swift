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

    /// `nil` only means the engine itself isn't running — Rust's own read
    /// never fails (a bad on-disk file degrades to defaults, reported via
    /// `SettingsModel.error`/`.notes`, not a thrown error).
    func settings() async -> SettingsModel? {
        guard let perch else { return nil }
        return await Task.detached(priority: .userInitiated) { perch.settings() }.value
    }

    /// Throws only when the write itself fails (e.g. the config directory
    /// isn't writable); a merely out-of-range value is clamped and reported
    /// back in the returned model's `notes`, not rejected.
    func saveSettings(_ s: Settings) async -> Result<SettingsModel, Error> {
        guard let perch else { return .failure(EngineUnavailable()) }
        return await Task.detached(priority: .userInitiated) {
            Result { try perch.saveSettings(s: s) }
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
