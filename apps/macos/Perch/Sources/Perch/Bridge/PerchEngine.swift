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
}

/// Rust calls this on its watcher thread; hop to the main actor before touching UI.
private final class Listener: PerchListener, @unchecked Sendable {
    private let deliver: @Sendable (PopoverModel) -> Void
    init(_ deliver: @escaping @Sendable (PopoverModel) -> Void) { self.deliver = deliver }
    func onModel(model: PopoverModel) { deliver(model) }
}
