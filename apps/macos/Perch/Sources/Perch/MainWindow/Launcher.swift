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
    static func run(_ command: TerminalCommand, terminal: String = "Terminal") throws {
        let dir = try scriptDirectory()
        let script = dir.appendingPathComponent("run-\(UUID().uuidString).command")
        let body = "#!/bin/sh\n\(command.shellLine)\n"
        do {
            try body.write(to: script, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        } catch {
            throw LauncherError.writeFailed(error.localizedDescription)
        }

        let config = NSWorkspace.OpenConfiguration()
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId(for: terminal))
            ?? NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal")
        else { throw LauncherError.noTerminal }
        NSWorkspace.shared.open([script], withApplicationAt: app, configuration: config)
    }

    private static func bundleId(for name: String) -> String {
        name == "iTerm2" ? "com.googlecode.iterm2" : "com.apple.Terminal"
    }

    /// Perch's own directory — the read-only promise about ~/.claude is unaffected.
    private static func scriptDirectory() throws -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Perch/commands", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        return base
    }
}
