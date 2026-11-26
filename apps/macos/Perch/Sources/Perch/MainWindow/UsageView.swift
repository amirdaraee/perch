import SwiftUI
import Charts
import PerchFFI

/// The Usage tab: hero stats with an optional burn-rate line, a daily chart
/// stacked by model, the top projects, and a per-model breakdown. Every
/// displayed string, tick label and tooltip line is finished in Rust; this
/// view lays them out, colours them and places the numbers Swift Charts needs
/// to scale. It never formats a number, a duration or a currency itself.
struct UsageView: View {
    let engine: PerchEngine

    @State private var model: UsageModel?
    @State private var error: String?
    /// The measure the person picked; `nil` means the model's default.
    @State private var measureKind: MeasureKind?
    /// The day under the pointer.
    @State private var hoveredDay: UInt32?

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
        // rebuilt whenever the sidebar selection moves away from Usage and
        // back (Sidebar.swift's detail pane renders a different view type
        // per selection), so a fresh instance — and a fresh load — is exactly
        // what re-selecting Usage should do.
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
            if let note = model.unpricedNote {
                HStack(spacing: 8) {
                    Text(note)
                        .font(.callout)
                        .foregroundStyle(Color.perchWaiting)
                    Button("Open Settings") {
                        (NSApp.delegate as? AppDelegate)?.showSettings()
                    }
                    .controlSize(.small)
                }
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

    // MARK: - Daily chart

    private func measure(_ chart: UsageChart) -> ChartMeasure? {
        chart.measures.first { $0.kind == measureKind } ?? chart.measures.first
    }

    @ViewBuilder
    private func dailyBlock(_ model: UsageModel) -> some View {
        let chart = model.chart
        if let current = measure(chart) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline) {
                    Text(chart.title).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
                    Spacer(minLength: 12)
                    if chart.measures.count > 1 {
                        Picker("", selection: Binding(
                            get: { current.kind },
                            set: { measureKind = $0 }
                        )) {
                            ForEach(chart.measures, id: \.kind) { m in
                                Text(m.title).tag(m.kind)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                    }
                }
                Text(current.caption).font(.caption).foregroundStyle(.secondary)
                dailyChart(chart, current)
                legend(chart)
            }
        }
    }

    private func dailyChart(_ chart: UsageChart, _ m: ChartMeasure) -> some View {
        let keys = chart.days.map { String($0.index) }
        let today = chart.days.first(where: \.isToday).map { String($0.index) }
        let active = Set(m.segments.map(\.day))
        // A 2pt seam between stacked layers, in data units, from an estimate
        // of the plot's height inside the frame below.
        let seam = m.yMax / 190 * 2
        return Chart {
            ForEach(Array(m.segments.enumerated()), id: \.offset) { _, seg in
                let isTop = !m.segments.contains { $0.day == seg.day && $0.start > seg.start }
                BarMark(
                    x: .value("Day", String(seg.day)),
                    yStart: .value("Value", seg.start + (seg.start > 0 ? seam / 2 : 0)),
                    yEnd: .value("Value", max(seg.end - (isTop ? 0 : seam / 2), seg.start + (seg.start > 0 ? seam / 2 : 0))),
                    width: .ratio(0.72)
                )
                .foregroundStyle(seriesColour(chart, seg.series))
                .opacity(String(seg.day) == today ? 0.5 : 1)
            }
            // A quiet day gets a dot on the baseline, so a gap reads as
            // "nothing happened" rather than as missing data.
            ForEach(chart.days.filter { !active.contains($0.index) }, id: \.index) { day in
                PointMark(x: .value("Day", String(day.index)), y: .value("Value", 0))
                    .symbolSize(10)
                    .foregroundStyle(Color.secondary.opacity(0.45))
            }
            if let hoveredDay {
                RuleMark(x: .value("Day", String(hoveredDay)))
                    .foregroundStyle(Color.secondary.opacity(0.35))
                    .lineStyle(StrokeStyle(lineWidth: 1))
                    .annotation(
                        position: .top,
                        spacing: 4,
                        overflowResolution: .init(x: .fit(to: .chart), y: .fit(to: .chart))
                    ) {
                        if Int(hoveredDay) < m.tooltips.count {
                            tooltip(chart, m.tooltips[Int(hoveredDay)])
                        }
                    }
            }
            PerchChartBaseline()
        }
        .chartXScale(domain: keys)
        .chartXAxis {
            AxisMarks(values: chart.days.filter { $0.axisLabel != nil }.map { String($0.index) }) { value in
                AxisTick()
                if let key = value.as(String.self), let day = chart.days.first(where: { String($0.index) == key }),
                   let label = day.axisLabel {
                    // The last label ends at its tick, so it never runs off
                    // the chart's trailing edge.
                    AxisValueLabel(anchor: day.index + 1 == UInt32(chart.days.count) ? .topTrailing : .top) {
                        Text(label).fixedSize()
                    }
                }
            }
        }
        .chartYScale(domain: 0...max(m.yMax, 1))
        .chartYAxis {
            AxisMarks(position: .leading, values: m.ticks.map(\.value)) { value in
                AxisGridLine().foregroundStyle(Color.secondary.opacity(0.15))
                if let v = value.as(Double.self), let tick = m.ticks.first(where: { $0.value == v }) {
                    AxisValueLabel { Text(tick.label).monospacedDigit() }
                }
            }
        }
        .chartOverlay { proxy in
            GeometryReader { geo in
                Rectangle().fill(.clear).contentShape(Rectangle())
                    .onContinuousHover { phase in
                        switch phase {
                        case .active(let point):
                            guard let frame = proxy.plotFrame else { return }
                            let x = point.x - geo[frame].origin.x
                            if let key: String = proxy.value(atX: x), let day = UInt32(key) {
                                hoveredDay = day
                            }
                        case .ended:
                            hoveredDay = nil
                        }
                    }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(m.caption)
        .accessibilityValue(m.tooltips.map { "\($0.title): \($0.total)" }.joined(separator: "; "))
        .frame(height: 230)
    }

    private func seriesColour(_ chart: UsageChart, _ series: UInt32) -> Color {
        let i = Int(series)
        return PerchModelPalette.color(i < chart.series.count ? chart.series[i].colour : 255)
    }

    private func tooltip(_ chart: UsageChart, _ t: DayTooltip) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(t.title).font(.caption.weight(.semibold))
            Text(t.total).font(.caption).monospacedDigit()
            ForEach(t.lines, id: \.self) { line in
                HStack(spacing: 6) {
                    RoundedRectangle(cornerRadius: 2).fill(seriesColour(chart, line.series)).frame(width: 8, height: 8)
                    Text(line.label).font(.caption2)
                    Spacer(minLength: 8)
                    Text(line.value).font(.caption2).monospacedDigit().foregroundStyle(.secondary)
                }
            }
        }
        .padding(8)
        .frame(minWidth: 160, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Color(nsColor: .separatorColor)))
        .fixedSize()
    }

    @ViewBuilder
    private func legend(_ chart: UsageChart) -> some View {
        if !chart.series.isEmpty {
            HStack(spacing: 14) {
                ForEach(chart.series, id: \.self) { s in
                    HStack(spacing: 5) {
                        RoundedRectangle(cornerRadius: 2).fill(PerchModelPalette.color(s.colour)).frame(width: 9, height: 9)
                        Text(s.name).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
            }
        }
    }

    // MARK: - Top projects

    @ViewBuilder
    private func topProjectsBlock(_ model: UsageModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(model.topProjectsTitle).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
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
                            RoundedRectangle(cornerRadius: 2)
                                .fill(PerchModelPalette.color(usage.colour))
                                .frame(width: 9, height: 9)
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
