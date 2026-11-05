import SwiftUI

/// The small uppercase caption that names a section of a card.
struct SectionHeader: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.caption2)
            .foregroundStyle(.secondary)
            .textCase(.uppercase)
    }
}
