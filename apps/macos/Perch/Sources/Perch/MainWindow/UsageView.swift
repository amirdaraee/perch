import SwiftUI
import Charts
import PerchFFI

/// The Usage tab: hero stats with an optional burn-rate line, 14 days of
/// token-class stacked bars, a 30-day top-projects ranking, and a per-model
/// breakdown (spec §9.3). Every displayed string is already finished on
/// `UsageModel` — `stat.value`, `stat.caption`, `bar.totalLabel`,
/// `p.tokensLabel`, `p.cost`, `m.tokens`, `m.cost`, `burnRate` — this view
/// only lays out, colours, and (for the daily chart) plots the numeric
/// fields Swift Charts needs to scale. It never formats a number, a
/// duration, or a currency itself.
struct UsageView: View {
    let engine: PerchEngine

    @State private var model: UsageModel?
    @State private var error: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                if let error {
                    Text(error).foregroundStyle(.secondary)
                } else if let model, !model.hasData {
                    // One honest line rather than four empty charts.
                    Text("No usage indexed yet").foregroundStyle(.secondary)
                } else if let model {
                    heroBlock(model)
                    dailyBlock(model)
                    topProjectsBlock(model)
                    byModelBlock(model)
                } else {
                    ProgressView().frame(maxWidth: .infinity, alignment: .center)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        // Plain `.task {}`, not `.task(id:)`: this view is torn down and
        // rebuilt whenever the Overview/Usage control switches away and
        // back (Sidebar.swift's detail pane renders a different view type
        // per tab), so a fresh instance — and a fresh load — is exactly
        // what re-selecting the tab should do.
        .task { await load() }
    }

    private func load() async {
        switch await engine.usage() {
        case .success(let m): model = m; error = nil
        case .failure(let e): error = e.localizedDescription
        }
    }

    // MARK: - Hero stats

    @ViewBuilder
    private func heroBlock(_ model: UsageModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 28) {
                ForEach(model.hero, id: \.self) { stat in
                    heroTile(stat)
                }
            }
            // `burnRate` is nil whenever Perch cannot project honestly —
            // omit the line entirely rather than showing a placeholder.
            if let burnRate = model.burnRate {
                Text(burnRate)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func heroTile(_ stat: HeroStat) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(stat.label).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            Text(stat.value).font(.title2.weight(.semibold)).monospacedDigit()
            if let caption = stat.caption {
                Text(caption).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    // MARK: - Daily stacked bars

    @ViewBuilder
    private func dailyBlock(_ model: UsageModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Last 14 days").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            if model.daily.isEmpty {
                Text("No daily usage yet").foregroundStyle(.secondary)
            } else {
                dailyChart(model.daily)
            }
        }
    }

    /// One `BarMark` per non-zero token class per day, explicitly stacked
    /// via `yStart`/`yEnd` (rather than `foregroundStyle(by:)`'s automatic
    /// stacking) so a small ~2pt gap can be carved out of each internal
    /// boundary per the spec's stacked-segment rule. `foregroundStyle(by:)`
    /// still drives the colour and the legend — only the geometry is manual.
    private func dailyChart(_ daily: [DailyBar]) -> some View {
        let segments = dailySegments(daily)
        let yMax = daily.map(dailyTotal).max() ?? 0
        // The gap is carved out of data units, sized from an *estimate* of
        // the plot area's pixel height inside the fixed frame below (chart
        // height minus Charts' own axis/padding reservation). Not pixel-
        // exact — Swift Charts doesn't expose the realized plot rect before
        // layout — but close enough to read as a deliberate ~2pt seam
        // rather than a hairline rounding error.
        let estimatedPlotHeight: Double = 190
        let gapValue = yMax > 0 ? (Double(yMax) / estimatedPlotHeight) * 2 : 0

        return Chart(segments) { segment in
            BarMark(
                x: .value("Day", segment.dayIndex),
                yStart: .value("Tokens", segment.yStart(gap: gapValue)),
                yEnd: .value("Tokens", segment.yEnd(gap: gapValue))
            )
            .foregroundStyle(by: .value("Class", segment.classLabel))
            .accessibilityLabel("\(segment.dayLabel), \(segment.classLabel)")
            .annotation(position: .top, alignment: .center, spacing: 2) {
                // `totalLabel` is Rust's finished string; shown once per
                // day, above its topmost rendered segment.
                if segment.isLast {
                    Text(segment.totalLabel)
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        .fixedSize()
                }
            }
        }
        // Fixed hue order, pinned explicitly rather than left to plotting
        // order — a legend with direct labels is always shown alongside.
        .chartForegroundStyleScale(
            domain: PerchChartPalette.order.map(\.label),
            range: PerchChartPalette.order.map(\.color)
        )
        .chartLegend(position: .bottom, spacing: 12)
        .chartXAxis {
            AxisMarks(values: axisDayIndices(daily)) { value in
                AxisTick()
                if let day = value.as(Int32.self), let label = dayLabel(for: day, in: daily) {
                    AxisValueLabel(label)
                }
            }
        }
        // No axis-drawn numbers: Charts' default y-axis labels would format
        // a raw token count itself, which Swift must never do. Recessive
        // grid lines only, no value labels, no dollars on this or any
        // second axis.
        .chartYAxis {
            AxisMarks(position: .leading) { _ in
                AxisGridLine().foregroundStyle(Color.secondary.opacity(0.15))
            }
        }
        .chartYScale(domain: 0...max(Double(yMax), 1))
        .frame(height: 220)
    }

    /// Four classes only — matches `TurnUsage::total_tokens()` and
    /// `bar.totalLabel`, both of which exclude thinking as a subset of
    /// output. The stacked bar's height must equal the labelled total.
    private func dailyTotal(_ bar: DailyBar) -> UInt64 {
        bar.input + bar.output + bar.cacheRead + bar.cacheWrite
    }

    private func dailySegments(_ daily: [DailyBar]) -> [DailySegment] {
        daily.flatMap { bar -> [DailySegment] in
            var cumulative: UInt64 = 0
            var rendered: [(label: String, start: UInt64, end: UInt64)] = []
            for entry in PerchChartPalette.order {
                let value = classValue(entry.key, in: bar)
                guard value > 0 else { continue }
                rendered.append((entry.label, cumulative, cumulative + value))
                cumulative += value
            }
            // The stacked height must equal the number `totalLabel` was
            // rendered from — this is exactly the double-counting bug the
            // four-class fix above closes (thinking used to push `cumulative`
            // past `dailyTotal(bar)`, so the bar drew taller than its own
            // annotation).
            assert(
                cumulative == dailyTotal(bar),
                "stacked bar height (\(cumulative)) must equal the labelled total (\(dailyTotal(bar)))"
            )
            return rendered.enumerated().map { index, seg in
                DailySegment(
                    dayIndex: bar.dayIndex,
                    dayLabel: bar.label,
                    classLabel: seg.label,
                    totalLabel: bar.totalLabel,
                    start: seg.start,
                    end: seg.end,
                    isFirst: index == 0,
                    isLast: index == rendered.count - 1
                )
            }
        }
    }

    /// Mapped by typed key, not display string — a typo in a string literal
    /// would silently yield 0 for a whole class. (Moving the labels
    /// themselves into Rust, so Swift never reinvents them, is backlogged.)
    private func classValue(_ key: PerchTokenClass, in bar: DailyBar) -> UInt64 {
        switch key {
        case .input: return bar.input
        case .output: return bar.output
        case .cacheRead: return bar.cacheRead
        case .cacheWrite: return bar.cacheWrite
        }
    }

    /// First, middle, and last day of the window — enough for context
    /// without crowding the axis with all 14 labels.
    private func axisDayIndices(_ daily: [DailyBar]) -> [Int32] {
        let days = daily.map(\.dayIndex).sorted()
        guard let first = days.first, let last = days.last else { return [] }
        guard days.count > 2 else { return days }
        return [first, days[days.count / 2], last]
    }

    private func dayLabel(for dayIndex: Int32, in daily: [DailyBar]) -> String? {
        daily.first { $0.dayIndex == dayIndex }?.label
    }

    // MARK: - Top projects

    @ViewBuilder
    private func topProjectsBlock(_ model: UsageModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Top projects (30 days)").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            if model.topProjects.isEmpty {
                Text("No project usage yet").foregroundStyle(.secondary)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(model.topProjects, id: \.self) { project in
                        HStack(spacing: 12) {
                            Text(project.name).fontWeight(.medium).lineLimit(1)
                            Spacer(minLength: 12)
                            Text(project.tokensLabel)
                                .foregroundStyle(.secondary)
                                .monospacedDigit()
                            Text(project.cost)
                                .foregroundStyle(.secondary)
                                .monospacedDigit()
                                .frame(minWidth: 60, alignment: .trailing)
                        }
                    }
                }
            }
        }
    }

    // MARK: - By model

    @ViewBuilder
    private func byModelBlock(_ model: UsageModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("By model").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            if model.byModel.isEmpty {
                Text("No model usage yet").foregroundStyle(.secondary)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(model.byModel, id: \.self) { usage in
                        HStack(spacing: 12) {
                            Text(usage.model).fontWeight(.medium).lineLimit(1)
                            Spacer(minLength: 12)
                            Text(usage.tokensLabel)
                                .foregroundStyle(.secondary)
                                .monospacedDigit()
                            Text(usage.cost)
                                .foregroundStyle(.secondary)
                                .monospacedDigit()
                                .frame(minWidth: 60, alignment: .trailing)
                        }
                    }
                }
            }
        }
    }
}

/// One rendered token-class segment of one day's stacked bar. Built once per
/// non-zero class so a day with, say, no cache-write tokens draws four
/// segments rather than five with an invisible sliver.
private struct DailySegment: Identifiable {
    let dayIndex: Int32
    let dayLabel: String
    let classLabel: String
    let totalLabel: String
    let start: UInt64
    let end: UInt64
    /// True for the segment closest to the baseline (no gap trimmed below it).
    let isFirst: Bool
    /// True for the segment closest to the bar's top (no gap trimmed above it,
    /// and where the day's `totalLabel` annotation is anchored).
    let isLast: Bool

    var id: String { "\(dayIndex)-\(classLabel)" }

    func yStart(gap: Double) -> Double {
        isFirst ? Double(start) : Double(start) + gap / 2
    }

    func yEnd(gap: Double) -> Double {
        let trimmed = isLast ? Double(end) : Double(end) - gap / 2
        return max(trimmed, yStart(gap: gap))
    }
}
