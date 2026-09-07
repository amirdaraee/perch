import ServiceManagement

/// Wraps `SMAppService.mainApp` — Perch's login-item registration.
///
/// The `launchAtLogin` field in `Settings` is Perch's own record of what it
/// last asked macOS for; it is not the same thing as what macOS actually has
/// registered right now — a user can remove a login item from System
/// Settings without Perch ever hearing about it, and a fresh install has
/// never registered anything no matter what a hand-edited `config.toml`
/// claims. `SettingsRootView`'s toggle reads `isRegistered` (never the
/// stored setting) precisely so it never shows "on" while nothing is
/// actually registered — a setting that lies about its own effect is worse
/// than no setting at all.
enum LoginItem {
    static var isRegistered: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// Registers or unregisters Perch as a login item. Throws whatever
    /// `SMAppService` throws (e.g. macOS refusing the request) — the caller
    /// is expected to surface that rather than silently persisting a stored
    /// setting that doesn't reflect what's actually true.
    static func setRegistered(_ enabled: Bool) throws {
        if enabled {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }
}
