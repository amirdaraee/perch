import AppKit
import SwiftUI
import PerchFFI

/// Where Perch keeps its own two files, what is in the index, the two buttons
/// that act on all of it, and About.
///
/// Everything above the buttons is read-only and finished: the sizes, the
/// counts and the ages are composed in `ui::advanced`, and this view renders
/// them verbatim. A fact Perch could not establish arrives as a dash rather
/// than a zero — "your index is empty" and "Perch could not look" are
/// different claims, and only one of them is ever true here.
///
/// Both buttons are honest about failing. A reindex that could not run says
/// so instead of redrawing the same numbers as though it had, and a reset
/// that could not be written changes nothing at all — it is one write of one
/// whole `Settings` in Rust, so there is no half-reset state to land in.
struct AdvancedPane: View {
    let engine: PerchEngine
    /// Bumped each time the settings window is (re)presented.
    let refreshToken: Int
    /// Runs the global reset through the window that owns the schema, so the
    /// panes behind this one adopt the result rather than going stale.
    let resetAllSettings: @MainActor () -> Void

    @State private var model: AdvancedModel?
    @State private var loadError: String?
    @State private var actionError: String?
    @State private var reindexing = false
    @State private var confirmingReset = false

    var body: some View {
        Group {
            if let model {
                content(model)
            } else if let loadError {
                Text(loadError)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: refreshToken) { await load() }
    }

    @ViewBuilder
    private func content(_ m: AdvancedModel) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let actionError {
                    Text(actionError)
                        .font(.callout)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.red.opacity(0.12))
                        .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                }

                ForEach(Array(m.groups.enumerated()), id: \.offset) { _, group in
                    factGroup(group, links: group.heading == aboutHeading ? m.links : [])
                }

                actions(m)
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .confirmationDialog(
            "Reset every setting?",
            isPresented: $confirmingReset,
            titleVisibility: .visible
        ) {
            Button("Reset all settings", role: .destructive) {
                actionError = nil
                resetAllSettings()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(m.resetWarning)
        }
    }

    // MARK: - Facts

    @ViewBuilder
    private func factGroup(_ group: FactGroup, links: [LinkRow]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(group.heading)
                .font(.caption2.weight(.semibold))
                .textCase(.uppercase)
                .foregroundStyle(.secondary)

            VStack(spacing: 0) {
                ForEach(Array(group.rows.enumerated()), id: \.offset) { index, row in
                    if index > 0 { Divider() }
                    factRow(row)
                }
                if !links.isEmpty {
                    Divider()
                    linkRow(links)
                }
            }
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Color(nsColor: .controlBackgroundColor))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(Color.primary.opacity(0.08))
            )
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func factRow(_ row: FactRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(row.label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 150, alignment: .leading)
            Text(row.value)
                .font(.callout)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 8)
            // Offered only where Rust found something actually there — a
            // Reveal that opens Finder on nothing is worse than no button.
            if let path = row.revealPath {
                Button("Reveal") { reveal(path) }
                    .buttonStyle(.link)
                    .accessibilityLabel("Reveal \(row.label) in Finder")
            }
            // Copies Rust's own `copyValue`, never the drawn text, which the
            // row may have truncated in the middle.
            if let text = row.copyValue {
                Button("Copy") { copy(text) }
                    .buttonStyle(.link)
                    .accessibilityLabel("Copy \(row.label)")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }

    /// The URLs are Rust's: the app's own Swift may not contain an absolute
    /// URL at all, which is the rule that keeps it unable to reach anything
    /// but this library.
    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func linkRow(_ links: [LinkRow]) -> some View {
        HStack(spacing: 14) {
            Text("Links")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 150, alignment: .leading)
            ForEach(Array(links.enumerated()), id: \.offset) { _, link in
                Button(link.label) { open(link.url) }
                    .buttonStyle(.link)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }

    // MARK: - The two buttons

    @ViewBuilder
    private func actions(_ m: AdvancedModel) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Index")
                    Spacer()
                    if reindexing {
                        ProgressView().controlSize(.small)
                    }
                    Button("Reindex now") { reindex() }
                        .disabled(reindexing)
                }
                Text(m.reindexHelp)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Divider()

            // Destructive, and at the foot of the pane rather than inline
            // among the readouts it would undo.
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text("Settings")
                    Spacer()
                    Button("Reset all settings…", role: .destructive) {
                        confirmingReset = true
                    }
                }
                Text(m.resetWarning)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Actions

    private func reindex() {
        reindexing = true
        actionError = nil
        Task {
            switch await engine.reindexNow() {
            case .success(let m):
                // Adopt what the run produced: the counts and the "last
                // written" line are this reindex's, not the previous ones
                // redrawn as if something had happened.
                model = m
            case .failure(let error):
                actionError = error.localizedDescription
            }
            reindexing = false
        }
    }

    private func reveal(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    private func open(_ url: String) {
        guard let target = URL(string: url) else { return }
        NSWorkspace.shared.open(target)
    }

    private func load() async {
        actionError = nil
        guard let m = await engine.advanced() else {
            loadError = EngineUnavailable().localizedDescription
            return
        }
        model = m
        loadError = nil
    }

    /// The heading Rust gives the About group. Matched rather than assumed
    /// by position, so reordering the groups in Rust cannot silently attach
    /// the links to the wrong one.
    private var aboutHeading: String { "About" }
}
