import SwiftUI
import PerchFFI

/// The sidebar's groups, in the sidebar's order, shared by the sidebar and the
/// overview grid so the two can never list projects under different headings.
enum ProjectGroups {
    static let ordered: [(title: String, group: ProjectGroup)] = [
        ("Pinned", .pinned),
        ("Active", .active),
        ("Recent", .recent),
        ("Archived", .archived),
    ]
}

/// Every project as a card, grouped as the sidebar groups them. Membership,
/// order and every string on a card are decided in Rust; this lays them out.
struct ProjectCardGrid: View {
    let projects: [ProjectRow]
    let onOpen: (Int64) -> Void

    private let columns = [GridItem(.adaptive(minimum: 260, maximum: 420), spacing: 12, alignment: .top)]

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            ForEach(ProjectGroups.ordered, id: \.title) { entry in
                let rows = projects.filter { $0.group == entry.group }
                if !rows.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(entry.title)
                            .font(.caption)
                            .fontWeight(.semibold)
                            .foregroundStyle(.secondary)
                            .textCase(.uppercase)
                        LazyVGrid(columns: columns, alignment: .leading, spacing: 12) {
                            ForEach(rows, id: \.id) { row in
                                ProjectCard(row: row) { onOpen(row.id) }
                            }
                        }
                    }
                }
            }
        }
    }
}

struct ProjectCard: View {
    let row: ProjectRow
    let open: () -> Void
    @State private var hovering = false
    /// The hovered day's sentence, shown in place of the stat line.
    @State private var hoveredDay: String?

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 8) {
                header
                // Reserve three lines whether or not there is a description,
                // so cards in one grid row keep their day cells aligned.
                Text(row.description ?? " ")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(3, reservesSpace: true)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HeatStrip(cells: row.recent, hovered: $hoveredDay)
                VStack(alignment: .leading, spacing: 2) {
                    Text(hoveredDay ?? row.statsLine)
                        .font(.caption)
                        .monospacedDigit()
                        .lineLimit(1)
                    Text(row.activityLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color(nsColor: .controlBackgroundColor))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(hovering ? Color.accentColor.opacity(0.6) : Color(nsColor: .separatorColor))
            )
            .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(row.path)
    }

    private var header: some View {
        HStack(spacing: 6) {
            Text(row.name)
                .font(.headline)
                .lineLimit(1)
            Spacer(minLength: 4)
            if let waiting = row.waitingLabel {
                Text(waiting)
                    .font(.caption2)
                    .fontWeight(.medium)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.perchWaiting.opacity(0.22)))
                    .foregroundStyle(Color.perchWaiting)
            }
            if let running = row.liveLabel {
                HStack(spacing: 4) {
                    Circle().fill(Color.perchWorking).frame(width: 6, height: 6)
                    Text(running).font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
    }
}
