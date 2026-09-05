import SwiftUI
import PerchFFI

/// The main window's content: a `Now` pane plus every project, grouped.
///
/// `refreshToken` is bumped by `MainWindowController.show()` each time the
/// window is (re)shown, so `.task(id:)` re-runs even though the hosting view
/// (and this view's identity) survives a close/reopen cycle. Without it a
/// SwiftUI `.task` only fires once per identity, and a reopened window would
/// keep showing whatever project list happened to be current when it first
/// loaded.
struct MainWindowRoot: View {
    let engine: PerchEngine
    let refreshToken: Int

    @State private var model: MainWindowModel?
    @State private var selection: Selection? = .now

    enum Selection: Hashable { case now, project(Int64) }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label("Now", systemImage: "dot.radiowaves.left.and.right").tag(Selection.now)
                if let model {
                    section("Pinned", .pinned, model)
                    section("Active", .active, model)
                    section("Recent", .recent, model)
                    section("Archived", .archived, model)
                }
            }
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 360)
        } detail: {
            detailPane
        }
        .task(id: refreshToken) { await reload() }
        .frame(minWidth: 820, minHeight: 520)
    }

    @ViewBuilder
    private func section(_ title: String, _ group: ProjectGroup, _ model: MainWindowModel) -> some View {
        let rows = model.projects.filter { $0.group == group }
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows, id: \.id) { row in
                    ProjectRowView(row: row).tag(Selection.project(row.id))
                }
            }
        }
    }

    @ViewBuilder
    private var detailPane: some View {
        switch selection {
        case .now, .none:
            NowPane(model: model)
        case .project(let id):
            ProjectDetailPane(engine: engine, projectId: id, onChanged: { await reload() })
        }
    }

    private func reload() async {
        model = await engine.mainWindow()
    }
}

struct ProjectRowView: View {
    let row: ProjectRow
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(row.name).fontWeight(.medium).lineLimit(1)
                if row.liveSessionCount > 0 {
                    Circle().fill(Color.perchWorking).frame(width: 7, height: 7)
                }
                Spacer()
            }
            Text(row.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(.vertical, 2)
    }
}

/// The `Now` pane reuses the popover's live model rather than inventing a second
/// one — same data, more room.
struct NowPane: View {
    let model: MainWindowModel?
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let err = model?.error {
                    Text(err).foregroundStyle(.secondary)
                }
                if let banner = model?.now.waitingBanner {
                    Text(banner).font(.headline).foregroundStyle(Color.perchWaiting)
                }
                if let live = model?.now.live, !live.isEmpty {
                    ForEach(live, id: \.id) { SessionRowView(row: $0) }
                } else {
                    Text("No sessions running").foregroundStyle(.secondary)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Stub — Task 9 replaces this with the real project detail pane (note
/// editing, sparkline, session history, terminal actions). Deliberately
/// minimal here so it's obvious this isn't the finished view.
struct ProjectDetailPane: View {
    let engine: PerchEngine
    let projectId: Int64
    let onChanged: () async -> Void

    var body: some View {
        Text(verbatim: "Project \(projectId)")
            .font(.title2)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
