import SwiftUI
import PerchFFI

/// The main window's content: Overview and Usage, then every project, grouped.
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
    /// Set only when this presentation was triggered by clicking a "waiting
    /// on you" notification — see `MainWindowController.show(selectingProjectId:)`.
    /// Applied once `reload()` has the project list; `nil` for an ordinary
    /// open.
    var selectProjectId: Int64? = nil

    @State private var model: MainWindowModel?
    /// Distinguishes "still loading" from "loaded and empty" — both render
    /// `model == nil` otherwise, and would be indistinguishable in the list.
    @State private var hasLoaded = false
    @State private var selection: Selection? = .overview
    /// Every place the user has been, in order, and where in that list the
    /// window is now — a browser's history, so Back and Forward retrace
    /// whatever mix of sidebar clicks and card clicks got here.
    @State private var history: [Selection] = [.overview]
    @State private var cursor = 0

    enum Selection: Hashable { case overview, usage, project(Int64) }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Label("Overview", systemImage: "square.grid.2x2").tag(Selection.overview)
                Label("Usage", systemImage: "chart.bar").tag(Selection.usage)
                if let model {
                    ForEach(ProjectGroups.ordered, id: \.title) { entry in
                        section(entry.title, entry.group, model)
                    }
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
                .toolbar {
                    ToolbarItemGroup(placement: .navigation) {
                        Button(action: goBack) {
                            Label("Back", systemImage: "chevron.left")
                        }
                        .keyboardShortcut("[", modifiers: .command)
                        .disabled(cursor == 0)
                        .help("Back")

                        Button(action: goForward) {
                            Label("Forward", systemImage: "chevron.right")
                        }
                        .keyboardShortcut("]", modifiers: .command)
                        .disabled(cursor >= history.count - 1)
                        .help("Forward")
                    }
                }
        }
        .onChange(of: selection) { _, new in record(new) }
        .task(id: refreshToken) { await reload() }
        .frame(minWidth: 820, minHeight: 520)
    }

    /// A step through history sets `selection` to the entry already under the
    /// cursor, which is how this tells it apart from a fresh visit: only a
    /// fresh visit drops the forward entries and appends.
    private func record(_ new: Selection?) {
        guard let new, new != history[cursor] else { return }
        history.removeSubrange((cursor + 1)...)
        history.append(new)
        cursor = history.count - 1
    }

    private func goBack() {
        guard cursor > 0 else { return }
        cursor -= 1
        selection = history[cursor]
    }

    private func goForward() {
        guard cursor < history.count - 1 else { return }
        cursor += 1
        selection = history[cursor]
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
        case .overview, .none:
            NowPane(engine: engine, projects: model?.projects ?? []) { id in
                selection = .project(id)
            }
        case .usage:
            UsageView(engine: engine)
        case .project(let id):
            ProjectDetailPane(engine: engine, projectId: id, onChanged: { await reload() })
                .id(id)
        }
    }

    private func reload() async {
        model = await engine.mainWindow()
        hasLoaded = true

        // A notification carries the project's id (`WaitingNotification.projectId`),
        // which is unique; the displayed directory name is not, and two
        // projects sharing one used to route to whichever the just-loaded
        // list happened to hold first. Nothing is matched or compared here
        // beyond checking the id is still in that list — a project deleted or
        // re-indexed away since the alert fired selects nothing, rather than
        // selecting something else.
        if let id = selectProjectId,
           model?.projects.contains(where: { $0.id == id }) == true {
            selection = .project(id)
        }
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
    /// The pull-loaded project list from `MainWindowRoot.reload()`, drawn as
    /// cards under whatever is running right now.
    let projects: [ProjectRow]
    let onOpenProject: (Int64) -> Void

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
                    // Each card already says what runs in its project, so the
                    // live sessions are not listed a second time above them.
                    if projects.isEmpty {
                        Text("No sessions running").foregroundStyle(.secondary)
                    }
                    ProjectCardGrid(projects: projects, onOpen: onOpenProject)
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
