import SwiftUI
import PerchFFI

/// A year of activity as a calendar grid. Rust decides every cell's level,
/// place and words; this sizes the cells to the width it gets and draws them.
/// Shared by every view that shows activity, so there is one implementation.
struct ActivityHeatmap: View {
    let model: Heatmap

    @State private var width: CGFloat = 0
    /// The hovered cell's sentence, shown under the grid so the detail never
    /// depends on telling shades apart.
    @State private var hovered: String?

    private let weekdayColumn: CGFloat = 30
    private let monthRow: CGFloat = 14
    /// Gap between cells, as a fraction of a cell.
    private let gapRatio: CGFloat = 0.25
    private let maxCell: CGFloat = 14

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // The cells are placed by offset, so the grid's frame is what
            // gives it a size; anchored top-left, or the offsets start mid-frame.
            grid
                .frame(
                    maxWidth: .infinity,
                    minHeight: monthRow + 7 * cell + 6 * gap,
                    maxHeight: monthRow + 7 * cell + 6 * gap,
                    alignment: .topLeading
                )
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
            Text(hovered ?? model.summary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, weekdayColumn)
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                if let note = model.trackingNote {
                    Text(note).font(.caption2).foregroundStyle(.tertiary)
                }
                Spacer(minLength: 8)
                legend
            }
            .padding(.leading, weekdayColumn)
        }
        .accessibilityElement(children: .contain)
    }

    private var columns: CGFloat { CGFloat(max(model.columns, 1)) }

    /// Whatever fits the width, up to `maxCell`.
    private var cell: CGFloat {
        let available = max(width - weekdayColumn, 0)
        let fitted = available / (columns + (columns - 1) * gapRatio)
        return max(min(fitted, maxCell), 2)
    }

    private var gap: CGFloat { cell * gapRatio }
    private var pitch: CGFloat { cell + gap }

    private var grid: some View {
        ZStack(alignment: .topLeading) {
            ForEach(model.months, id: \.self) { month in
                Text(month.label)
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .offset(x: weekdayColumn + CGFloat(month.index) * pitch, y: 0)
            }
            ForEach(model.weekdays, id: \.self) { day in
                Text(day.label)
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .frame(height: cell)
                    .offset(x: 0, y: monthRow + CGFloat(day.index) * pitch)
            }
            ForEach(model.cells, id: \.self) { c in
                HeatSwatch(level: c.level, beforeTracking: c.beforeTracking)
                    .frame(width: cell, height: cell)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        if inside { hovered = c.label } else if hovered == c.label { hovered = nil }
                    }
                    .help(c.label)
                    .accessibilityLabel(c.label)
                    .offset(
                        x: weekdayColumn + CGFloat(c.column) * pitch,
                        y: monthRow + CGFloat(c.row) * pitch
                    )
            }
        }
    }

    private var legend: some View {
        HStack(spacing: 3) {
            HeatSwatch(level: 0, beforeTracking: true).frame(width: 9, height: 9)
            Text(model.legendUntracked).font(.caption2).foregroundStyle(.secondary)
                .padding(.trailing, 6)
            Text(model.legendLess).font(.caption2).foregroundStyle(.secondary)
            ForEach(0..<5, id: \.self) { level in
                HeatSwatch(level: UInt8(level), beforeTracking: false).frame(width: 9, height: 9)
            }
            Text(model.legendMore).font(.caption2).foregroundStyle(.secondary)
        }
        .fixedSize()
        .accessibilityHidden(true)
    }
}

/// One day's square, shared by every heatmap so a shade means the same
/// thing wherever it appears. Quiet days are a faint fill; days before
/// tracking are an outline only, so they don't read as quiet ones.
struct HeatSwatch: View {
    let level: UInt8
    let beforeTracking: Bool

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 2, style: .continuous)
        if beforeTracking {
            shape.strokeBorder(Color.secondary.opacity(0.25), lineWidth: 1)
        } else if level == 0 {
            shape.fill(Color.secondary.opacity(0.15))
        } else {
            shape.fill(Color.perchWorking.opacity(Self.opacity[Int(min(level, 4))]))
        }
    }

    private static let opacity: [Double] = [0, 0.35, 0.55, 0.78, 1.0]
}

/// A single row of days, as on a project card: the cells share the width,
/// up to the Overview's cell size. `hovered` receives the cell's sentence.
struct HeatStrip: View {
    let cells: [HeatCell]
    @Binding var hovered: String?

    private let gapRatio: CGFloat = 0.25
    private let maxCell: CGFloat = 14
    @State private var width: CGFloat = 0

    var body: some View {
        HStack(spacing: gap) {
            ForEach(cells, id: \.self) { c in
                HeatSwatch(level: c.level, beforeTracking: c.beforeTracking)
                    .frame(width: cell, height: cell)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        if inside { hovered = c.label } else if hovered == c.label { hovered = nil }
                    }
                    .help(c.label)
                    .accessibilityLabel(c.label)
            }
        }
        .frame(maxWidth: .infinity, minHeight: cell, maxHeight: cell, alignment: .leading)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .accessibilityElement(children: .contain)
    }

    private var count: CGFloat { CGFloat(max(cells.count, 1)) }
    private var cell: CGFloat {
        let fitted = width / (count + (count - 1) * gapRatio)
        return max(min(fitted, maxCell), 2)
    }
    private var gap: CGFloat { cell * gapRatio }
}
