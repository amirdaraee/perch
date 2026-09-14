import SwiftUI
import PerchFFI

/// One live session. Named `SessionRowView` (not `SessionRow`) to avoid
/// colliding with the generated `SessionRow` record it displays.
struct SessionRowView: View {
    let row: SessionRow
    @Environment(\.perchDensity) private var density

    var body: some View {
        HStack(alignment: .top, spacing: density.rowSpacing) {
            Circle().fill(dotColor).frame(width: 8, height: 8).padding(.top, 5)
            VStack(alignment: .leading, spacing: density.lineSpacing) {
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
                // Empty means the user turned "Show project folder" off — a
                // different thing from the em dash that means "not known",
                // which is why this tests for blankness and never for a dash.
                // Rust blanks the field; drawing or not drawing it is all
                // that is decided here.
                if !row.folder.isEmpty {
                    Text(row.folder)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        // A path's tail (the checkout, the worktree) is the
                        // half that distinguishes it, so lose the middle.
                        .truncationMode(.middle)
                }
            }
        }
        .perchCard(.row)
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
