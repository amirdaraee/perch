import AppKit
import PerchFFI

enum LauncherError: LocalizedError {
    case noTerminal
    case writeFailed(String)
    var errorDescription: String? {
        switch self {
        case .noTerminal: "No terminal application could be found to run the command."
        case .writeFailed(let m): "Could not prepare the command: \(m)"
        }
    }
}

/// Runs a Rust-composed command in the user's terminal.
///
/// Running a command without triggering an Automation permission prompt is
/// done by writing a single-use script into Perch's own application-support
/// directory and asking `NSWorkspace` to open it with the terminal app.
/// AppleScript would need `NSAppleEventsUsageDescription` and a permission
/// dialog; this needs neither. The script never touches the Claude Code
/// directory — Perch's read-only promise about `~/.claude` covers Swift too.
///
/// `command.shellLine` arrives from `perch-core::actions` already
/// POSIX-single-quoted for every value it embeds; it is used verbatim here,
/// with no additional escaping or interpolation.
enum Launcher {
    static func run(_ command: TerminalCommand) throws {
        let dir = try scriptDirectory()
        sweep(dir)

        let script = dir.appendingPathComponent("run-\(UUID().uuidString).command")
        let body = "#!/bin/sh\n\(command.shellLine)\n"
        do {
            try body.write(to: script, atomically: true, encoding: .utf8)
        } catch {
            throw LauncherError.writeFailed(error.localizedDescription)
        }
        do {
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        } catch {
            // The write succeeded but the chmod didn't: don't leave a
            // non-executable script behind for the sweep to find later.
            try? FileManager.default.removeItem(at: script)
            throw LauncherError.writeFailed(error.localizedDescription)
        }

        let config = NSWorkspace.OpenConfiguration()
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal")
        else { throw LauncherError.noTerminal }
        NSWorkspace.shared.open([script], withApplicationAt: app, configuration: config)
    }

    /// Perch's own directory — the read-only promise about ~/.claude is unaffected.
    private static func scriptDirectory() throws -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Perch/commands", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base
    }

    /// Every script is single-use and nothing else ever deletes it, so this
    /// directory would otherwise grow without bound. Sweep leftovers older
    /// than a minute — long enough that a script the terminal hasn't
    /// finished launching yet is never touched, short enough that the
    /// directory never accumulates.
    private static func sweep(_ dir: URL) {
        let cutoff = Date().addingTimeInterval(-60)
        guard let items = try? FileManager.default.contentsOfDirectory(
            at: dir, includingPropertiesForKeys: [.contentModificationDateKey]
        ) else { return }
        for item in items where item.pathExtension == "command" {
            let modified = (try? item.resourceValues(forKeys: [.contentModificationDateKey]))?
                .contentModificationDate
            if let modified, modified < cutoff {
                try? FileManager.default.removeItem(at: item)
            }
        }
    }
}
