import SwiftUI
import PerchFFI

/// Up to a few recently-ended sessions, listed below the live ones.
struct RecentCard: View {
    let rows: [RecentRow]
    @Environment(\.perchDensity) private var density

    var body: some View {
        VStack(alignment: .leading, spacing: density.rowSpacing / 2) {
            SectionHeader(title: "Recent")
            ForEach(rows, id: \.id) { r in
                HStack {
                    Circle().fill(Color.perchBackground).frame(width: 8, height: 8)
                    Text(r.project).lineLimit(1)
                    Spacer()
                    Text(r.endedLine)
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
            }
        }
        .perchCard()
    }
}
