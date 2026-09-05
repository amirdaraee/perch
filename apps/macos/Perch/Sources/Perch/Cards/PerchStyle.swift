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

extension Color {
    /// Convenience for the fixed hex values the design spec hands us —
    /// keeps `PerchChartPalette` below readable as the literal hex it was
    /// validated at, rather than hand-converted RGB fractions.
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

/// The Usage view's categorical chart palette — five hues in a fixed order,
/// validated (dataviz skill, `references/palette.md`, dark-surface
/// categorical slots 1-5) for lightness band, chroma floor, colour-vision-
/// deficiency separation, and contrast. Never substitute or reorder these:
/// a chart's colour identity must stay stable across renders, and the
/// values already passed every check.
///
/// `order` is both the stacking order (input closest to the baseline) and
/// the legend order; index into it by token class rather than cycling.
enum PerchChartPalette {
    static let input = Color(hex: 0x3987e5)
    static let output = Color(hex: 0xd95926)
    static let cacheRead = Color(hex: 0x199e70)
    static let cacheWrite = Color(hex: 0xc98500)
    static let thinking = Color(hex: 0xd55181)

    static let order: [(label: String, color: Color)] = [
        ("Input", input),
        ("Output", output),
        ("Cache read", cacheRead),
        ("Cache write", cacheWrite),
        ("Thinking", thinking),
    ]
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
