import AppKit
import UserNotifications
import PerchFFI

/// Hands finished `WaitingNotification` strings to `UNUserNotificationCenter`.
///
/// Everything that decides whether to notify, when, and what to say already
/// happened in Rust (`notify.rs`) before a `WaitingNotification` ever exists —
/// this type composes nothing of its own. It does exactly three things:
/// request authorization (only when asked to, never on its own), deliver a
/// finished `title`/`body` verbatim, and hand a click back to whoever wants
/// to route it.
///
/// `@MainActor` because `UNUserNotificationCenter` and AppKit both want the
/// main actor; the two `UNUserNotificationCenterDelegate` methods below are
/// `nonisolated` (the protocol itself isn't actor-isolated) and hop onto the
/// actor themselves before touching anything — the same shape
/// `PerchEngine`'s `Listener` uses for `onModel`/`onNotifications`.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    /// The clicked notification's `sessionId`/`projectId` — a plain mirror of
    /// the two `WaitingNotification` fields that went into its `userInfo`,
    /// not anything decided here. `projectId` is what identifies the project
    /// to open: it is unique, unlike the displayed directory name, which two
    /// projects can share. `nil` when Rust could not resolve one (the
    /// directory isn't indexed) — the caller then opens the window without
    /// selecting anything rather than guessing.
    var onClicked: ((_ sessionId: String, _ projectId: Int64?) -> Void)?

    override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    /// Requested only when the user flips "Notify when a session is waiting
    /// on you" ON in Settings — never at launch. A permission prompt before
    /// the app has shown its worth is the fastest way to be denied for good.
    /// Returns whether the app is now authorized, so the caller can make the
    /// toggle reflect a denial instead of sitting on while delivering
    /// nothing.
    func requestAuthorization() async -> Bool {
        do {
            return try await UNUserNotificationCenter.current()
                .requestAuthorization(options: [.alert, .sound])
        } catch {
            return false
        }
    }

    /// The *current* authorization state, without prompting — used to paint
    /// the Settings toggle correctly on load, including the case where the
    /// user granted access in a previous run and later revoked it from
    /// System Settings without ever touching Perch's toggle again.
    func authorizationStatus() async -> UNAuthorizationStatus {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// Delivers each item's `title`/`body` exactly as Rust composed them —
    /// no reformatting, no truncation, no recomposition, and no dedupe: a
    /// `WaitingNotification` only exists because `notify.rs`'s edge-triggered
    /// decision already fired on it, so nothing here second-guesses that.
    /// `userInfo` carries just enough for a click to be routed back to its
    /// session/project. The request's own identifier is a fresh UUID per
    /// call rather than anything derived from `sessionId` — keying it on
    /// session identity would itself be a dedupe decision, and that decision
    /// already lives in Rust, not here.
    func deliver(_ items: [WaitingNotification]) {
        let center = UNUserNotificationCenter.current()
        for item in items {
            let content = UNMutableNotificationContent()
            content.title = item.title
            content.body = item.body
            content.sound = .default
            // Only what a click needs to be routed. `projectId` is omitted
            // entirely when Rust resolved none, so its absence at click time
            // is the same "no project to open" it was at delivery time.
            var userInfo: [String: Any] = ["sessionId": item.sessionId]
            if let projectId = item.projectId {
                userInfo["projectId"] = NSNumber(value: projectId)
            }
            content.userInfo = userInfo
            let request = UNNotificationRequest(
                identifier: UUID().uuidString,
                content: content,
                trigger: nil
            )
            center.add(request)
        }
    }

    /// Without this, a foreground app's own notifications are suppressed by
    /// default — but Perch is a menu-bar accessory app whose whole point is
    /// surfacing a session that needs attention even while, say, the main
    /// window is already open, so the banner must still show.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    /// Rust calls `deliver(_:)` on the main actor already (`PerchEngine`
    /// hops before calling it), but a click arrives from AppKit/UserNotifications
    /// on whatever thread the system chooses — hop before touching
    /// `onClicked`, the same rule as every other callback into this app.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let info = response.notification.request.content.userInfo
        let sessionId = info["sessionId"] as? String ?? ""
        // Absent (or, from some older delivered notification still sitting in
        // Notification Centre, unreadable) means no project to select — never
        // fall back to a name-shaped guess.
        let projectId = (info["projectId"] as? NSNumber)?.int64Value
        Task { @MainActor [weak self] in
            self?.onClicked?(sessionId, projectId)
        }
        completionHandler()
    }
}
