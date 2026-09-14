import SwiftUI
import PerchFFI

/// One session's facts, drawn as the head of its submenu.
///
/// Every label and every value arrives finished from Rust — including the em
/// dash that means "Perch does not know". A fact the user asked not to see
/// (tokens, cost) is not a row with an empty value here: Rust leaves it out
/// of `detail` altogether, so there is nothing to filter for.
struct SessionDetailCard: View {
    let rows: [DetailRow]

    var body: some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 3) {
            ForEach(rows.indices, id: \.self) { i in
                GridRow {
                    Text(rows[i].label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(rows[i].value)
                        .font(.caption)
                        .lineLimit(1)
                        // A path's tail (the checkout, the worktree) and an
                        // id's ends are the halves that distinguish them, so
                        // lose the middle — the same choice the popover row
                        // makes for its folder line.
                        .truncationMode(.middle)
                        .frame(maxWidth: Metrics.valueWidth, alignment: .leading)
                }
            }
        }
        .padding(.horizontal, Metrics.horizontalPadding)
        .padding(.vertical, Metrics.verticalPadding)
    }

    private enum Metrics {
        /// Wide enough for a session id in full, narrow enough that the
        /// submenu does not end up wider than the popover it hangs off.
        static let valueWidth: CGFloat = 260
        static let horizontalPadding: CGFloat = 14
        static let verticalPadding: CGFloat = 8
    }
}
