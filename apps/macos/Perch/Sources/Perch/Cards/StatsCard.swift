import SwiftUI
import PerchFFI

/// The three headline numbers: window, week, and today's cost. Every string
/// (including "—" for absent data) comes from `Stats`; this view only lays
/// out and colours what it is given.
struct StatsCard: View {
    let stats: Stats

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            stat("Window", stats.windowTokens)
            stat("Week", stats.weekTokens)
            VStack(alignment: .leading, spacing: 2) {
                Text("24h").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
                Text(stats.dayCost).font(.title3.weight(.semibold)).monospacedDigit()
                if stats.hasData && stats.estimated {
                    Text("est")
                        .font(.caption2)
                        .foregroundStyle(Color.perchWaiting)
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .overlay(
                            Capsule()
                                .strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [3, 2]))
                                .foregroundStyle(Color.perchWaiting)
                        )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .perchCard(verticalPadding: 10)
    }

    private func stat(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            Text(value).font(.title3.weight(.semibold)).monospacedDigit()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
