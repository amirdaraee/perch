import SwiftUI
import PerchFFI

/// Up to a few recently-ended sessions, listed below the live ones.
struct RecentCard: View {
    let rows: [RecentRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Recent").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            ForEach(rows, id: \.id) { r in
                HStack {
                    Circle().fill(Color(red: 0.39, green: 0.39, blue: 0.40)).frame(width: 8, height: 8)
                    Text(r.project).lineLimit(1)
                    Spacer()
                    Text(r.tokens == "—" ? "ended \(r.endedAgo) ago" : "\(r.tokens) · ended \(r.endedAgo) ago")
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .frame(width: 360, alignment: .leading)
    }
}
