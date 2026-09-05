import SwiftUI

/// A one- or two-line placeholder row: startup errors, index trouble, the
/// waiting banner, and "no sessions running" all share this shape.
struct EmptyCard: View {
    let title: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).foregroundStyle(.secondary)
            if let detail { Text(detail).font(.caption2).foregroundStyle(.tertiary).lineLimit(2) }
        }
        .perchCard()
    }
}
