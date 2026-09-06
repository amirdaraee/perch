import Foundation
import PerchFFI

/// Owns the Rust `Perch` object and republishes its model on the main actor.
/// Nothing in Swift reads Claude Code's files; this is the only doorway.
@MainActor
final class PerchEngine: ObservableObject {
    @Published private(set) var model: PopoverModel?
    @Published private(set) var startupError: String?

    private var perch: Perch?
    private var listener: Listener?

    func start() {
        do {
            let p = try Perch(configDir: nil)
            let l = Listener { [weak self] m in
                Task { @MainActor in self?.model = m }
            }
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
}

struct EngineUnavailable: LocalizedError {
    var errorDescription: String? { "Perch's engine is not running." }
}

/// Rust calls this on its watcher thread; hop to the main actor before touching UI.
private final class Listener: PerchListener, @unchecked Sendable {
    private let deliver: @Sendable (PopoverModel) -> Void
    init(_ deliver: @escaping @Sendable (PopoverModel) -> Void) { self.deliver = deliver }
    func onModel(model: PopoverModel) { deliver(model) }

    // No-op for now: Task 6 added this method to the `PerchListener` trait
    // (a breaking change to a foreign trait), so this conformance must exist
    // for the tree to build. Task 9 (notification delivery) fills this in.
    func onNotifications(items: [WaitingNotification]) {}
}
