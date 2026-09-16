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

    /// Local UI-only projection of `NotifyOverride`'s three cases onto the
    /// segmented control's selection. `NotifyOverride.custom` carries a
    /// minutes payload that doesn't fit a plain `Hashable` picker tag, so the
    /// number is handled separately by `customMinutesBinding`.
    private enum NotifyMode: Hashable {
        case `default`, custom, off
    }
    /// Derived from `detail`, never separately mutated — same shape as
    /// `pinnedBinding`/`archivedBinding` above, and for the same reason: a
    /// `setNotifyOverride` call that throws must leave the segmented control
    /// showing what the project's override actually still is, not whatever
    /// was optimistically selected before the write failed.
    private var notifyMode: NotifyMode {
        switch detail?.notify {
        case .off: return .off
        case .custom: return .custom
        case .default, .none: return .default
        }
    }
    /// Fallback source for the stepper while `notifyMode != .custom`, and
    /// `nil` until there is one: written only by `seedCustomMinutes` (a fresh
    /// `.custom` override from the server) or by dialing the stepper itself.
    /// While it is `nil`, `customMinutesBinding` reads the *global* default's
    /// own minute count out of `ProjectDetail.notifyDefaultMinutes` — the
    /// number behind the "Default — waits N minutes" label shown right beside
    /// this control. The seed is a product decision, so it comes from Rust
    /// like every other one; a literal here would silently drift the moment
    /// the Rust default moved.
    ///
    /// `customMinutesBinding` prefers `detail`'s own value whenever the
    /// override truly is `.custom` — that's what makes a stepper edit that
    /// fails snap back to the last *persisted* number instead of keeping the
    /// optimistic one — and only falls back to this field when `detail`
    /// doesn't carry a custom value at all, which is what keeps a dialed-in
    /// number alive across a Default/Off detour.
    @State private var customMinutes: UInt32?

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
                notifySection(detail)
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

    /// The milestone's distinguishing decision: the per-project notification
    /// override lives here, on the project the user is already looking at —
    /// not as a row in the settings window. Default/Custom/Off; the custom
    /// stepper stays visible but disabled unless Custom is selected, matching
    /// Task 7's settings window (dependent controls grey out, they don't
    /// vanish).
    @ViewBuilder
    private func notifySection(_ detail: ProjectDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Notifications").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)

            Picker("Notifications", selection: notifyModeBinding(detail)) {
                Text("Default").tag(NotifyMode.default)
                Text("Custom").tag(NotifyMode.custom)
                Text("Off").tag(NotifyMode.off)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 280)

            // Shows what "Default" currently means so the user isn't
            // choosing blind — `notifyDefaultLabel` is already a finished
            // sentence fragment composed in Rust, never assembled here.
            if notifyMode == .default {
                Text(detail.notifyDefaultLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Stepper(value: customMinutesBinding(detail), in: 1...240) {
                Text(customNotifyLabel(minutes: customMinutesBinding(detail).wrappedValue))
            }
            .disabled(notifyMode != .custom)
        }
    }

    @ViewBuilder
    private func sparkline(_ detail: ProjectDetail) -> some View {
        if !detail.sparkline.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Last 14 days").font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
                Chart {
                    ForEach(detail.sparkline, id: \.dayIndex) { point in
                        BarMark(
                            x: .value("Day", point.dayIndex),
                            y: .value("Tokens", point.tokens)
                        )
                        .foregroundStyle(Color.perchWorking)
                        // `label` is Rust's already-final day label — never formatted here.
                        .accessibilityLabel(point.label)
                    }
                    PerchChartBaseline()
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

    /// Takes `detail` rather than reading the optional `@State` one, because
    /// switching to Custom has to send a number and the number it sends is
    /// whatever the stepper is showing — which, before the user has dialled
    /// anything, is the global default carried on `detail` itself.
    private func notifyModeBinding(_ detail: ProjectDetail) -> Binding<NotifyMode> {
        Binding(
            // No optimistic assignment here — `notifyMode` reads straight
            // from `detail`, so a `setNotifyOverride` that throws leaves the
            // picker showing exactly what it showed before the tap, same as
            // `pinnedBinding`/`archivedBinding`.
            get: { notifyMode },
            set: { newMode in
                let minutes = customMinutesBinding(detail).wrappedValue
                Task { await setNotify(mode: newMode, minutes: minutes) }
            }
        )
    }

    /// Also takes `detail`, so the un-dialled seed has a definite source:
    /// `notifyDefaultMinutes`, the global threshold Rust composed the
    /// neighbouring "Default — waits N minutes" label from. Nothing here
    /// invents a starting number, and nothing parses that label to recover
    /// one.
    private func customMinutesBinding(_ detail: ProjectDetail) -> Binding<UInt32> {
        Binding(
            get: {
                // Prefer `detail`'s own number whenever the override truly is
                // `.custom` — that's what makes a stepper edit that fails
                // snap back to the last *persisted* minutes instead of
                // keeping the optimistic one. Only outside `.custom` (where
                // `detail` has no minutes to read) does this fall back to the
                // locally-tracked seed, which is what keeps a dialed-in
                // number alive across a Default/Off detour — and, before
                // anything has been dialled, to the global default.
                if case .custom(let afterMinutes) = detail.notify { return afterMinutes }
                return customMinutes ?? detail.notifyDefaultMinutes
            },
            set: { newValue in
                customMinutes = newValue
                // The stepper is disabled outside `.custom` (SwiftUI blocks
                // the interaction that would call this setter); the guard is
                // defence-in-depth only, matching this file's existing
                // `guard d.id == projectId` in `apply(_:)`.
                guard notifyMode == .custom else { return }
                Task { await setNotify(mode: .custom, minutes: newValue) }
            }
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
        // Read fresh at the moment of launch, not cached from whenever this
        // view first loaded — a `preferredTerminal` change (from this
        // window, or a hand-edited config.toml) is picked up on the very
        // next launch with no extra plumbing.
        guard let bundleId = await engine.preferredTerminalBundleId() else {
            actionError = EngineUnavailable().localizedDescription
            return
        }
        do {
            try Launcher.run(command, bundleId: bundleId)
        } catch {
            self.actionError = error.localizedDescription
        }
    }

    private func load() async {
        loadError = nil
        switch await engine.projectDetail(projectId) {
        case .success(let d):
            detail = d
            noteDraft = d.note
            seedCustomMinutes(from: d.notify)
        case .failure(let e):
            loadError = e.localizedDescription
        }
    }

    /// `notifyMode` reads straight from `detail`, so it needs no seeding.
    /// `customMinutes` is the one bit of notify state actually kept in local
    /// `@State` (as a fallback for while the override isn't `.custom` — see
    /// its declaration above), so it's what this seeds from a fresh/refreshed
    /// override, same moment `detail`/`noteDraft` get seeded.
    private func seedCustomMinutes(from override: NotifyOverride) {
        guard case .custom(let afterMinutes) = override else { return }
        customMinutes = afterMinutes
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
            // Cleared here, not only set in the failure arm below: without
            // this, a rejected pin toggle leaves its banner on screen through
            // every later edit that succeeds, asserting a failure that is no
            // longer true. `SettingsRootView.load()` carries the same fix for
            // the same reason.
            actionError = nil
            detail = d
            noteDraft = d.note
            // `set_notify_override` clamps the custom threshold in Rust
            // (1...240); `notifyMode` already reads straight from `detail`,
            // so only `customMinutes`'s fallback needs reseeding here — this
            // is what makes a clamped value display as clamped.
            seedCustomMinutes(from: d.notify)
            await onChanged()
        case .failure(let e):
            // Deliberately no re-seed here: `notifyMode` and
            // `customMinutesBinding.get`'s primary path both read straight
            // from `detail`, which a failure never touches — so a rejected
            // `setNotifyOverride` already leaves the notify control showing
            // exactly the pre-edit, still-persisted state without anything
            // extra to restore.
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

    private func setNotify(mode: NotifyMode, minutes: UInt32) async {
        guard let detail else { return }
        let override: NotifyOverride
        switch mode {
        case .default: override = .default
        case .off: override = .off
        case .custom: override = .custom(afterMinutes: minutes)
        }
        await apply(await engine.setNotifyOverride(projectId: detail.id, override: override))
    }

    private func rename() async {
        guard let detail else { isRenaming = false; return }
        let trimmed = renameDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        isRenaming = false
        guard !trimmed.isEmpty, trimmed != detail.name else { return }
        await apply(await engine.renameProject(projectId: detail.id, name: trimmed))
    }
}
