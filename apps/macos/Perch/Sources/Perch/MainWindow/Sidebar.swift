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
    // `@ObservedObject`, not `let`: the sidebar itself only pulls the project
    // list (see `reload()` below), but it hosts `NowPane`, whose entire
    // premise is liveness, and it needs to re-render when `engine.model` or
    // `engine.startupError` change to pass those through.
    @ObservedObject var engine: PerchEngine
    let refreshToken: Int

    @State private var model: MainWindowModel?
    /// Distinguishes "still loading" from "loaded and empty" — both render
    /// `model == nil` otherwise, and would be indistinguishable in the list.
    @State private var hasLoaded = false
    @State private var selection: Selection? = .now
    @State private var tab: DetailTab = .overview

    enum Selection: Hashable { case now, project(Int64) }

    /// The detail pane's top-level control (spec §9.2). Independent of
    /// `selection`: switching to Usage and back to Overview keeps whichever
    /// sidebar row was selected, since Usage shows account-wide totals
    /// rather than anything scoped to one project.
    enum DetailTab: Hashable { case overview, usage }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label("Now", systemImage: "dot.radiowaves.left.and.right").tag(Selection.now)
                if let model {
                    section("Pinned", .pinned, model)
                    section("Active", .active, model)
                    section("Recent", .recent, model)
                    section("Archived", .archived, model)
                } else if !hasLoaded {
                    ProgressView().frame(maxWidth: .infinity, alignment: .center)
                } else if let reason = engine.startupError {
                    // `engine.mainWindow()` returned `nil` because the engine
                    // itself never started — an empty sidebar here would be
                    // indistinguishable from a healthy, empty install.
                    Text(reason).font(.caption).foregroundStyle(.secondary)
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
        VStack(spacing: 0) {
            Picker("View", selection: $tab) {
                Text("Overview").tag(DetailTab.overview)
                Text("Usage").tag(DetailTab.usage)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 220)
            .padding(.horizontal, 20)
            .padding(.top, 16)
            .padding(.bottom, 8)

            switch tab {
            case .overview:
                overviewPane
            case .usage:
                UsageView(engine: engine)
            }
        }
    }

    @ViewBuilder
    private var overviewPane: some View {
        switch selection {
        case .now, .none:
            NowPane(engine: engine)
        case .project(let id):
            ProjectDetailPane(engine: engine, projectId: id, onChanged: { await reload() })
                .id(id)
        }
    }

    private func reload() async {
        model = await engine.mainWindow()
        hasLoaded = true
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
///
/// Reads `engine.model` (the live, listener-pushed `PopoverModel`) directly,
/// not a snapshot captured when `MainWindowRoot.reload()` last ran: pull-only
/// is right for the project list, whose totals only need to be as fresh as
/// the last time the window opened, but wrong here, where the whole point is
/// showing a session start, finish, or block while the window stays open.
struct NowPane: View {
    @ObservedObject var engine: PerchEngine
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let reason = engine.startupError {
                    // The engine never started at all — more fundamental
                    // than an index-open failure, so it wins.
                    Text(reason).foregroundStyle(.secondary)
                } else if let model = engine.model {
                    // The specific reason (e.g. "could not open index:
                    // {e}"), not a generic placeholder — `PopoverModel.error`
                    // is exactly that, unlike `MainWindowModel.error`, which
                    // only ever says "index unavailable".
                    if let err = model.error {
                        Text(err).foregroundStyle(.secondary)
                    }
                    if let banner = model.waitingBanner {
                        Text(banner).font(.headline).foregroundStyle(Color.perchWaiting)
                    }
                    if !model.live.isEmpty {
                        ForEach(model.live, id: \.id) { SessionRowView(row: $0) }
                    } else {
                        Text("No sessions running").foregroundStyle(.secondary)
                    }
                } else {
                    // Before the engine's first emit lands — distinct from a
                    // genuinely empty "no sessions running".
                    ProgressView().frame(maxWidth: .infinity, alignment: .center)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
