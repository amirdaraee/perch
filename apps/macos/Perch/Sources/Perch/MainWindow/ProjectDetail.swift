import SwiftUI
import Charts
import PerchFFI

/// A single project in full: note, totals, sparkline, session history, and
/// the two actions the user asked for — Resume a past session and Open a
/// fresh one — plus pin, archive, and rename. Every displayed string
/// (`detailLine`, `subtitle`, `tokens`, `cost`, `started`, `duration`,
/// `sessionCount`) is already final in `ProjectDetail`; this view lays out
/// and colours what it is given, and never formats a number or duration.
struct ProjectDetailPane: View {
    let engine: PerchEngine
    let projectId: Int64
    let onChanged: () async -> Void

    @State private var detail: ProjectDetail?
    @State private var loadError: String?
    @State private var actionError: String?

    @State private var noteDraft: String = ""
    @FocusState private var noteFocused: Bool

    @State private var isRenaming = false
    @State private var renameDraft: String = ""

    var body: some View {
        Group {
            if let detail {
                content(detail)
            } else if let loadError {
                Text(loadError)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: projectId) { await load() }
        // The note otherwise only saves on focus loss, but `.id(id)` at this
        // pane's call site (`Sidebar.swift`) tears the whole pane — and its
        // `@State noteDraft` — down the instant the selection changes,
        // before a text field can ever resign focus. `saveNoteIfChanged`
        // already no-ops when the draft matches the loaded note, so this
        // never writes on a plain tab switch or a switch back to the same
        // project.
        .onDisappear {
            Task { await saveNoteIfChanged() }
        }
    }

    // MARK: - Layout

    @ViewBuilder
    private func content(_ detail: ProjectDetail) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header(detail)

                if let actionError {
                    Text(actionError)
                        .font(.callout)
                        .foregroundStyle(Color.perchWaiting)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.perchWaiting.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
                }

                if !detail.pathExists {
                    Text("This project's folder can't be found, so terminal actions are disabled.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }

                totals(detail)
                noteEditor(detail)
                sparkline(detail)
                sessionList(detail)
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func header(_ detail: ProjectDetail) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if isRenaming {
                    TextField("Project name", text: $renameDraft)
                        .textFieldStyle(.roundedBorder)
                        .font(.title2)
                        .onSubmit { Task { await rename() } }
                    Button("Save") { Task { await rename() } }
                    Button("Cancel") { isRenaming = false }
                } else {
                    Text(detail.name).font(.title2).fontWeight(.semibold)
                    Button {
                        renameDraft = detail.name
                        isRenaming = true
                    } label: {
                        Image(systemName: "pencil")
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
                Toggle("Pinned", isOn: pinnedBinding)
                    .toggleStyle(.button)
                Toggle("Archived", isOn: archivedBinding)
                    .toggleStyle(.button)
            }

            Text(detail.path).font(.caption).foregroundStyle(.secondary)

            Button("Open new session") {
                Task { await launch { await engine.openCommand(cwd: detail.path) } }
            }
            .disabled(!detail.pathExists)
        }
    }

    private func totals(_ detail: ProjectDetail) -> some View {
        HStack(spacing: 24) {
            stat("Sessions", detail.sessionCount)
            stat("Tokens", detail.tokens)
            stat("Cost", detail.cost)
        }
    }

    private func stat(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            Text(value).font(.title3.weight(.semibold)).monospacedDigit()
        }
    }

    @ViewBuilder
    private func noteEditor(_ detail: ProjectDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Note").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            TextEditor(text: $noteDraft)
                .font(.body)
                .frame(minHeight: 70, maxHeight: 150)
                .focused($noteFocused)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.25)))
                .onChange(of: noteFocused) { _, focused in
                    if !focused { Task { await saveNoteIfChanged() } }
                }
        }
    }

    @ViewBuilder
    private func sparkline(_ detail: ProjectDetail) -> some View {
        if !detail.sparkline.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Last 14 days").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
                Chart(detail.sparkline, id: \.dayIndex) { point in
                    BarMark(
                        x: .value("Day", point.dayIndex),
                        y: .value("Tokens", point.tokens)
                    )
                    .foregroundStyle(Color.perchWorking)
                    // `label` is Rust's already-final day label — never formatted here.
                    .accessibilityLabel(point.label)
                }
                .chartXAxis {
                    // A handful of labelled ticks give the bars a day, without
                    // crowding 14 of them onto one axis.
                    AxisMarks(values: axisDayIndices(detail)) { value in
                        AxisTick()
                        if let day = value.as(Int32.self), let label = dayLabel(for: day, in: detail) {
                            AxisValueLabel(label)
                        }
                    }
                }
                .chartYAxis(.hidden)
                .frame(height: 70)
            }
        }
    }

    /// First, middle, and last day of the sparkline — enough for context
    /// without crowding the axis with all 14 labels.
    private func axisDayIndices(_ detail: ProjectDetail) -> [Int32] {
        let days = detail.sparkline.map(\.dayIndex).sorted()
        guard let first = days.first, let last = days.last else { return [] }
        guard days.count > 2 else { return days }
        return [first, days[days.count / 2], last]
    }

    private func dayLabel(for dayIndex: Int32, in detail: ProjectDetail) -> String? {
        detail.sparkline.first { $0.dayIndex == dayIndex }?.label
    }

    @ViewBuilder
    private func sessionList(_ detail: ProjectDetail) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Sessions").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            if detail.sessions.isEmpty {
                Text("No sessions yet").foregroundStyle(.secondary)
            } else {
                ForEach(detail.sessions, id: \.id) { session in
                    sessionRow(session, detail: detail)
                    Divider()
                }
            }
        }
    }

    private func sessionRow(_ session: SessionHistoryRow, detail: ProjectDetail) -> some View {
        HStack(alignment: .top, spacing: 9) {
            if session.isLive {
                Circle().fill(Color.perchWorking).frame(width: 8, height: 8).padding(.top, 5)
            }
            VStack(alignment: .leading, spacing: 1) {
                HStack {
                    Text(session.name).fontWeight(.medium).lineLimit(1)
                    Spacer()
                    Text(session.started).font(.caption).foregroundStyle(.secondary)
                }
                Text(session.detailLine).font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
            }
            Spacer(minLength: 12)
            Button("Resume") {
                Task {
                    await launch { await engine.resumeCommand(sessionId: session.id, cwd: detail.path) }
                }
            }
            .disabled(!detail.pathExists)
        }
        .padding(.vertical, 4)
    }

    // MARK: - Bindings

    private var pinnedBinding: Binding<Bool> {
        Binding(
            get: { detail?.pinned ?? false },
            set: { newValue in Task { await setPinned(newValue) } }
        )
    }

    private var archivedBinding: Binding<Bool> {
        Binding(
            get: { detail?.archived ?? false },
            set: { newValue in Task { await setArchived(newValue) } }
        )
    }

    // MARK: - Actions

    /// Every terminal launch funnels through here so failures surface
    /// consistently — a launch that silently does nothing is the worst
    /// outcome (spec §7). `make` returns `nil` only when the engine itself
    /// isn't running; that must surface too, not silently no-op.
    private func launch(_ make: () async -> TerminalCommand?) async {
        actionError = nil
        guard let command = await make() else {
            actionError = EngineUnavailable().localizedDescription
            return
        }
        do { try Launcher.run(command) } catch { self.actionError = error.localizedDescription }
    }

    private func load() async {
        loadError = nil
        switch await engine.projectDetail(projectId) {
        case .success(let d):
            detail = d
            noteDraft = d.note
        case .failure(let e):
            loadError = e.localizedDescription
        }
    }

    /// Adopts the refreshed model an edit method returns, rather than
    /// re-fetching or guessing what changed, and tells the sidebar to
    /// refresh so a pin/archive edit can move the project between groups.
    ///
    /// These edits run as independent, uncancelled `Task`s (not scoped to
    /// `.task(id:)`), so a late response can still arrive after the user has
    /// switched to a different project. What actually closes that race is
    /// `.id(id)` on this pane's call site in `Sidebar.swift`: switching
    /// projects gives the pane a fresh identity (and fresh `@State`), so a
    /// stale response has no live `detail`/`noteDraft` left to overwrite.
    /// `guard d.id == projectId` below is defence-in-depth only — as the
    /// code stands today it is a tautology (every caller passes
    /// `projectId: detail.id`, and `detail` only ever comes from a fetch of
    /// `projectId`), not the mechanism that protects against the race. Do
    /// not remove `.id(id)` on the strength of this guard alone.
    private func apply(_ result: Result<ProjectDetail, Error>) async {
        switch result {
        case .success(let d):
            guard d.id == projectId else { return }
            detail = d
            noteDraft = d.note
            await onChanged()
        case .failure(let e):
            actionError = e.localizedDescription
        }
    }

    private func saveNoteIfChanged() async {
        guard let detail, noteDraft != detail.note else { return }
        await apply(await engine.setNote(projectId: detail.id, note: noteDraft))
    }

    private func setPinned(_ pinned: Bool) async {
        guard let detail else { return }
        await apply(await engine.setPinned(projectId: detail.id, pinned: pinned))
    }

    private func setArchived(_ archived: Bool) async {
        guard let detail else { return }
        await apply(await engine.setArchived(projectId: detail.id, archived: archived))
    }

    private func rename() async {
        guard let detail else { isRenaming = false; return }
        let trimmed = renameDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        isRenaming = false
        guard !trimmed.isEmpty, trimmed != detail.name else { return }
        await apply(await engine.renameProject(projectId: detail.id, name: trimmed))
    }
}
