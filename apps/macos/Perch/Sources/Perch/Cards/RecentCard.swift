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
                    // `name` is the session's own title, falling back to its
                    // project only when it has none. Drawing `project`
                    // directly — as this did — made three finished sessions
                    // in one project read as the same word three times, and
                    // made the indexed titles look absent when they were not.
                    Text(r.name).lineLimit(1)
                    Spacer()
                    Text(r.endedLine)
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
            }
        }
        .perchCard()
    }
}
