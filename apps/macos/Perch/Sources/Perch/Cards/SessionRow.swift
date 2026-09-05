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
                    .foregroundStyle(row.status == .waiting ? Color(red: 1, green: 0.84, blue: 0.04) : .secondary)
                Text(detailLine)
                    .font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .frame(width: 360, alignment: .leading)
    }

    private var detailLine: String {
        [
            row.project,
            row.kind,
            row.version.isEmpty ? nil : "v\(row.version)",
            row.tokens == "—" ? nil : "\(row.tokens) · \(row.cost)",
        ]
        .compactMap { $0 }
        .joined(separator: " · ")
    }

    private var dotColor: Color {
        switch row.status {
        case .waiting: Color(red: 1, green: 0.84, blue: 0.04)
        case .working: Color(red: 0.19, green: 0.82, blue: 0.35)
        case .idle: Color(red: 0.19, green: 0.82, blue: 0.35).opacity(0.55)
        case .background: Color(red: 0.39, green: 0.39, blue: 0.40)
        }
    }
}
