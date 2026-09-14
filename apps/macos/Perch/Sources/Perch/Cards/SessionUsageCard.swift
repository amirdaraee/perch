import Charts
import SwiftUI
import PerchFFI

/// What one session has spent, drawn under its facts in its own submenu: the
/// four token classes broken out and totalled, the models that spent them
/// when there was more than one, and the shape of the session's activity over
/// its own lifetime.
///
/// Nothing here is composed. Every heading, every label, every value and
/// every caption on the chart's axis arrives finished from Rust — including
/// the sentence that stands in for all of it when there is nothing recorded,
/// and the emptiness that means the user turned these numbers off. This view
/// scales bars and places text, and makes no decision about what any of it
/// says.
struct SessionUsageCard: View {
    let usage: SessionUsage

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            heading(usage.heading)

            if let note = usage.note {
                // The one case that is a sentence rather than a table: no
                // turns recorded, or no index to read them from. Rust decides
                // which, and says so in words rather than in zeroes.
                Text(note)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: SubmenuMetrics.valueWidth, alignment: .leading)
            }

            if !usage.classes.isEmpty {
                DetailRowsGrid(rows: usage.classes)
            }

            if !usage.byModel.isEmpty {
                heading(usage.modelsHeading)
                DetailRowsGrid(rows: usage.byModel)
            }

            if !usage.chart.isEmpty {
                heading(usage.chartCaption)
                chart
            }
        }
        .padding(.horizontal, SubmenuMetrics.horizontalPadding)
        .padding(.vertical, SubmenuMetrics.verticalPadding)
    }

    @ViewBuilder
    private func heading(_ text: String) -> some View {
        if !text.isEmpty {
            Text(text)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
        }
    }

    private var chart: some View {
        Chart(usage.chart, id: \.index) { point in
            BarMark(
                x: .value("Slice", point.index),
                y: .value("Tokens", point.tokens)
            )
            .foregroundStyle(Color.perchWorking)
            // `label` is Rust's already-final caption for where in the
            // session's life this slice falls — never formatted here.
            .accessibilityLabel(point.label)
        }
        .chartXAxis {
            // First, middle and last only: sixteen captions would not fit
            // inside a menu, and three are enough to place a bar in the
            // session's lifetime.
            AxisMarks(values: axisIndices) { value in
                AxisTick()
                if let index = value.as(Int32.self), let label = label(for: index) {
                    AxisValueLabel(label)
                }
            }
        }
        .chartYAxis(.hidden)
        .frame(width: SubmenuMetrics.valueWidth, height: Metrics.chartHeight)
    }

    private var axisIndices: [Int32] {
        let indices = usage.chart.map(\.index)
        guard let first = indices.first, let last = indices.last else { return [] }
        guard indices.count > 2 else { return indices }
        return [first, indices[indices.count / 2], last]
    }

    private func label(for index: Int32) -> String? {
        usage.chart.first { $0.index == index }?.label
    }

    private enum Metrics {
        /// Tall enough to read a shape out of, short enough that the submenu
        /// still fits beside the popover it hangs off.
        static let chartHeight: CGFloat = 52
    }
}
