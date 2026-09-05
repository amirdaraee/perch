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

/// The daily stacked bar's token classes. Typed so Swift maps a `DailyBar`
/// field to a chart segment by case, not by matching a display string —
/// a typo in a string literal would silently render as zero.
///
/// Deliberately excludes thinking: `TurnUsage::total_tokens()`
/// (`crates/perch-core/src/model.rs`) excludes it too, since thinking tokens
/// are a subset of output, not an additional billable class. `DailyBar`
/// still carries `thinking` as real data; it is just never stacked.
enum PerchTokenClass: CaseIterable {
    case input, output, cacheRead, cacheWrite
}

/// The Usage view's categorical chart palette — four hues in a fixed order,
/// validated (dataviz skill, `references/palette.md`, dark-surface
/// categorical slots 1-4) for lightness band, chroma floor, colour-vision-
/// deficiency separation, and contrast (≥3:1 in both light and dark). Never
/// substitute or reorder these: a chart's colour identity must stay stable
/// across renders, and the values already passed every check.
///
/// Cache write is `0xb87a00`, not the originally shipped `0xc98500` — the
/// dataviz validator found the old value missed light-mode contrast at
/// 2.99:1; `0xb87a00` passes all six checks in both appearances, so no
/// appearance-aware variant is needed.
///
/// `order` is both the stacking order (input closest to the baseline) and
/// the legend order; index into it by token class rather than cycling.
enum PerchChartPalette {
    static let input = Color(hex: 0x3987e5)
    static let output = Color(hex: 0xd95926)
    static let cacheRead = Color(hex: 0x199e70)
    static let cacheWrite = Color(hex: 0xb87a00)

    static let order: [(label: String, key: PerchTokenClass, color: Color)] = [
        ("Input", .input, input),
        ("Output", .output, output),
        ("Cache read", .cacheRead, cacheRead),
        ("Cache write", .cacheWrite, cacheWrite),
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
