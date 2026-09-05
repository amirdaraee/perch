import SwiftUI

/// Colours and layout shared by every menu card, kept in one place so a
/// future tweak can't drift the shade or width apart between cards.
extension Color {
    /// Waiting sessions, the `est` badge, and anything else amber.
    static let perchWaiting = Color(red: 1, green: 0.84, blue: 0.04)
    /// Working sessions; idle sessions reuse this at reduced opacity.
    static let perchWorking = Color(red: 0.19, green: 0.82, blue: 0.35)
    /// Background/ended sessions.
    static let perchBackground = Color(red: 0.39, green: 0.39, blue: 0.40)
}

private enum PerchCardMetrics {
    /// The popover's fixed width (brief: ~360 pt, widened from the mockup's 340).
    static let width: CGFloat = 360
    static let horizontalPadding: CGFloat = 14
}

/// The common menu-card shape: full popover width, shared horizontal padding,
/// and a per-card vertical padding (section cards are looser than session rows).
struct PerchCardStyle: ViewModifier {
    var verticalPadding: CGFloat

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, PerchCardMetrics.horizontalPadding)
            .padding(.vertical, verticalPadding)
            .frame(width: PerchCardMetrics.width, alignment: .leading)
    }
}

extension View {
    func perchCard(verticalPadding: CGFloat = 8) -> some View {
        modifier(PerchCardStyle(verticalPadding: verticalPadding))
    }
}
