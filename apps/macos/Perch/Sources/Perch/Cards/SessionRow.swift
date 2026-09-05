import SwiftUI
import PerchFFI

/// One live session. Named `SessionRowView` (not `SessionRow`) to avoid
/// colliding with the generated `SessionRow` record it displays.
struct SessionRowView: View {
    let row: SessionRow

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Circle().fill(dotColor).frame(width: 8, height: 8).padding(.top, 5)
            VStack(alignment: .leading, spacing: 1) {
                HStack {
                    Text(row.name).fontWeight(.medium).lineLimit(1)
                    Spacer()
                    Text(row.elapsed).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
                Text(row.statusLabel)
                    .font(.caption)
                    .foregroundStyle(row.status == .waiting ? Color.perchWaiting : .secondary)
                Text(row.detailLine)
                    .font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
        }
        .perchCard(verticalPadding: 6)
    }

    private var dotColor: Color {
        switch row.status {
        case .waiting: .perchWaiting
        case .working: .perchWorking
        case .idle: .perchWorking.opacity(0.55)
        case .background: .perchBackground
        }
    }
}
