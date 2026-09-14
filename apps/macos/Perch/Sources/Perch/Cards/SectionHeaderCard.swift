import SwiftUI

/// The small uppercase caption that names a popover section. The live
/// sections draw theirs as a menu item of its own, because their rows are
/// separate items; `RecentCard` draws the same label inside itself, since its
/// rows are one card.
struct SectionHeader: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.caption2)
            .foregroundStyle(.secondary)
            .textCase(.uppercase)
    }
}

/// `SectionHeader` as a standalone card, for the sections whose rows are
/// individual menu items.
struct SectionHeaderCard: View {
    let title: String

    var body: some View {
        SectionHeader(title: title)
            .frame(maxWidth: .infinity, alignment: .leading)
            .perchCard(.row)
    }
}
