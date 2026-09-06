import SwiftUI
import PerchFFI

/// Answers one question: *why isn't my session showing up?* Reachable
/// directly from the settings window's tab bar — not hidden behind a debug
/// toggle — because it exists for someone already having trouble.
///
/// Every string here (`verdictLabel` above all) is already composed in Rust,
/// e.g. "ignored: pid 4821 is `zsh`, not `claude`". This view renders it
/// verbatim; it never reformats or re-derives a message from the structured
/// fields alongside it.
struct DiagnosticsView: View {
    let engine: PerchEngine
    /// Bumped by `SettingsWindowController.show()` each time the settings
    /// window is (re)presented. Without keying `.task` on this, reopening
    /// the window after visiting this tab once would keep showing whatever
    /// snapshot was current the first time — exactly wrong for the one pane
    /// whose job is answering "why isn't my session showing up *right now*?"
    let refreshToken: Int

    @State private var model: DiagnosticsModel?
    @State private var loadError: String?

    var body: some View {
        Group {
            if let model {
                content(model)
            } else if let loadError {
                Text(loadError)
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
    private func content(_ model: DiagnosticsModel) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                group("Claude Code") {
                    row("Config directory", model.configDir)
                    row("Detected via", model.configDirSource)
                    row("Sessions directory", model.sessionsDir)
                }

                group("Perch Settings") {
                    row("Settings file", model.settingsPath)
                    row("Loaded", model.settingsLoaded ? "Yes" : "No — using defaults")
                }

                group("Index") {
                    row("Index path", model.indexPath)
                    row("Sessions indexed", model.indexSessions)
                    row("Turns indexed", model.indexTurns)
                    row("Last indexed", model.lastIndexed)
                }

                group("Session Records") {
                    if model.records.isEmpty {
                        Text("No session records found.").foregroundStyle(.secondary)
                    } else {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(Array(model.records.enumerated()), id: \.offset) { _, record in
                                recordRow(record)
                            }
                        }
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func group<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption2).foregroundStyle(.secondary).textCase(.uppercase)
            content()
        }
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 120, alignment: .leading)
            Text(value)
                .font(.callout)
                .textSelection(.enabled)
        }
    }

    private func recordRow(_ record: RecordRow) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(record.file)
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .textSelection(.enabled)
            Text(record.verdictLabel)
                .font(.callout)
                .foregroundStyle(isAccepted(record.verdict) ? Color.perchWorking : Color.perchWaiting)
        }
    }

    private func isAccepted(_ verdict: RecordVerdict) -> Bool {
        if case .accepted = verdict { return true }
        return false
    }

    private func load() async {
        guard let m = await engine.diagnostics() else {
            loadError = EngineUnavailable().localizedDescription
            return
        }
        model = m
    }
}
