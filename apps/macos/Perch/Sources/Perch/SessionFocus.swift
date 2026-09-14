import AppKit
import Darwin

/// Which application is running a session, and bringing it forward.
///
/// A session's pid is `claude` itself — a command-line process with no
/// windows, which the window server has never heard of. What the user wants
/// brought forward is whatever terminal they launched it from, so this walks
/// the process tree upward until it reaches a pid that macOS recognises as an
/// application.
///
/// Walking the tree is done with `sysctl(KERN_PROC_PID)`, which is in the SDK
/// and needs no permission and no dependency. Raising the one *window or tab*
/// the session lives in would need Accessibility, which Perch has
/// deliberately never requested — so this brings the application forward and
/// stops there, which is exactly what the action's title (composed in Rust)
/// promises.
@MainActor
enum SessionFocus {
    /// The application that owns `pid`, or `nil` when the walk reaches the
    /// top without finding one — a session started by `launchd`, by a remote
    /// shell, or inside a terminal that has already quit while its child
    /// lives on.
    static func owningApplication(of pid: Int32) -> NSRunningApplication? {
        // Bounded rather than "until pid 1": a corrupt or recycled parent
        // chain that loops would otherwise hang the menu that is asking.
        // Sixteen hops is far more than any real terminal → shell → claude
        // chain, which is three.
        var current = pid
        for _ in 0..<16 {
            guard current > 1 else { return nil }
            if let app = application(for: current) { return app }
            guard let parent = parentPid(of: current), parent != current else { return nil }
            current = parent
        }
        return nil
    }

    /// Bring `pid`'s owning application forward. Returns false when nothing
    /// could be resolved — the same condition that leaves the menu action
    /// disabled, re-checked here because the process tree can change between
    /// the menu being built and the item being clicked.
    @discardableResult
    static func activateApplication(of pid: Int32) -> Bool {
        guard let app = owningApplication(of: pid) else { return false }
        return app.activate()
    }

    /// `NSRunningApplication` will answer for plenty of pids that are not
    /// applications in any sense a user would recognise, so a bundle
    /// identifier is required as well, and `.prohibited` (an agent with no
    /// Dock presence and nothing to bring forward) is rejected — activating
    /// one of those would look to the user like the menu item did nothing.
    private static func application(for pid: Int32) -> NSRunningApplication? {
        guard
            let app = NSRunningApplication(processIdentifier: pid),
            app.bundleIdentifier != nil,
            app.activationPolicy != .prohibited
        else { return nil }
        return app
    }

    /// One hop up the process tree. `nil` when the process has exited between
    /// the previous hop and this one, which is ordinary rather than an error.
    private static func parentPid(of pid: Int32) -> Int32? {
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        guard sysctl(&mib, u_int(mib.count), &info, &size, nil, 0) == 0, size > 0 else {
            return nil
        }
        let parent = info.kp_eproc.e_ppid
        return parent > 0 ? parent : nil
    }
}
