import AppKit
import Charts
import SwiftUI
import PerchFFI

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

/// One colour per model, indexed by the colour Rust assigns from the
/// all-time ranking, so a model looks the same in every chart and list.
/// The dataviz reference palette's first seven categorical slots, each
/// stepped separately for light and dark, in its validated order: adjacent
/// pairs pass colour-vision-deficiency separation on Perch's own window
/// surfaces in both appearances. Four light steps sit under 3:1 against the
/// light surface, so the chart always names models in its legend and tooltip.
/// Index 7 and above is "Other models", a neutral grey.
enum PerchModelPalette {
    private static let light: [UInt32] = [0x257de0, 0xf47230, 0x00b894, 0xf2a600, 0xee7fac, 0x008a17, 0x6a4ce2]
    private static let dark: [UInt32] = [0x338ced, 0xe76624, 0x00a888, 0xd28e05, 0xe24d8b, 0x0aac23, 0x9887ef]

    static func color(_ index: UInt8) -> Color {
        let i = Int(index)
        guard i < light.count else { return Color.secondary.opacity(0.55) }
        return Color(nsColor: NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark[i] : light[i]
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255,
                alpha: 1
            )
        })
    }
}

/// A hairline along a bar chart's floor, so an empty slice still reads as a
/// slice with nothing in it rather than as a gap in the chart.
struct PerchChartBaseline: ChartContent {
    var body: some ChartContent {
        RuleMark(y: .value("Tokens", 0))
            .lineStyle(StrokeStyle(lineWidth: 0.5))
            .foregroundStyle(Color.secondary.opacity(0.6))
    }
}

private enum PerchCardMetrics {
    /// The popover's fixed width (brief: ~360 pt, widened from the mockup's 340).
    static let width: CGFloat = 360
    static let horizontalPadding: CGFloat = 14
}

/// Which shape of card is being drawn. Each kind has its own vertical padding
/// — section cards breathe more than session rows — and each density has its
/// own table of the three.
enum PerchCardKind {
    case section
    case row
    case stats
}

/// The pixel table one `RowDensity` selects. The *choice* is the model's; the
/// measurements are drawing, which is the shell's half of the bargain. Nothing
/// here reads a setting: the metrics arrive from `PopoverModel.rowDensity` via
/// the environment, so the menu bar cannot disagree with the settings window
/// about how tight a row is.
struct PerchDensityMetrics {
    let section: CGFloat
    let row: CGFloat
    let stats: CGFloat
    /// Gap between a row's dot and its text column.
    let rowSpacing: CGFloat
    /// Gap between the stacked lines inside one row.
    let lineSpacing: CGFloat

    static let comfortable = PerchDensityMetrics(
        section: 8, row: 6, stats: 10, rowSpacing: 9, lineSpacing: 1
    )
    /// Roughly two-thirds the vertical air of `comfortable`, which is enough
    /// to read as tighter at a glance without the lines touching.
    static let compact = PerchDensityMetrics(
        section: 5, row: 2, stats: 6, rowSpacing: 7, lineSpacing: 0
    )

    func padding(for kind: PerchCardKind) -> CGFloat {
        switch kind {
        case .section: section
        case .row: row
        case .stats: stats
        }
    }
}

extension RowDensity {
    var metrics: PerchDensityMetrics {
        switch self {
        case .comfortable: .comfortable
        case .compact: .compact
        }
    }
}

private struct PerchDensityKey: EnvironmentKey {
    /// Matches `Settings::default().row_density`, so a card hosted before the
    /// first model arrives draws at the shipped density rather than a second
    /// one invented here.
    static let defaultValue = PerchDensityMetrics.comfortable
}

extension EnvironmentValues {
    var perchDensity: PerchDensityMetrics {
        get { self[PerchDensityKey.self] }
        set { self[PerchDensityKey.self] = newValue }
    }
}

/// The common menu-card shape: full popover width, shared horizontal padding,
/// and a vertical padding chosen by the card's kind at the current density.
struct PerchCardStyle: ViewModifier {
    var kind: PerchCardKind
    @Environment(\.perchDensity) private var density

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, PerchCardMetrics.horizontalPadding)
            .padding(.vertical, density.padding(for: kind))
            .frame(width: PerchCardMetrics.width, alignment: .leading)
    }
}

extension View {
    func perchCard(_ kind: PerchCardKind = .section) -> some View {
        modifier(PerchCardStyle(kind: kind))
    }
}
