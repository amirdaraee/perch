import AppKit
import SwiftUI
import PerchFFI

/// Everything a schema pane needs from the window hosting it, handed in
/// rather than reached for. This view owns no settings state of its own: every
/// value it draws comes from the `SettingsPane` it was given, and every edit
/// leaves through one of these closures, which is what keeps a control from
/// mutating anything optimistically and then having nothing to revert to when
/// the write fails.
@MainActor
struct SchemaPaneActions {
    /// Store one value under one key. The host serializes these and adopts
    /// the schema that comes back.
    var write: @MainActor (SettingKey, SettingValue) -> Void
    /// Restore this pane's settings to their factory values.
    var reset: @MainActor (PaneId) -> Void
    /// Run a row's action button. Rows carrying one have no key.
    var perform: @MainActor (SettingRow) -> Void
    /// Disabled for a reason Rust cannot know: macOS refusing to deliver
    /// notifications leaves `waiting_enabled = true` in the file — Rust
    /// rightly calls the dependent rows enabled, and the window rightly
    /// greys them, because nothing below that switch can happen.
    var additionallyDisabled: @MainActor (SettingRow) -> Bool
    /// The handful of rows whose displayed truth lives in macOS rather than
    /// in `config.toml` — the login item, and the notification switch macOS
    /// can refuse. `nil` for every other key, which is nearly all of them.
    var systemToggle: @MainActor (SettingKey) -> SystemToggle?
}

/// A toggle whose ON state is macOS's answer rather than the stored setting,
/// so it can never read ON over a thing that is not actually happening.
struct SystemToggle {
    var isOn: Bool
    var isDisabled: Bool
    var set: @MainActor (Bool) -> Void
}

/// Draws any pane the schema describes: one SwiftUI control per `Control`
/// case, each with the row's label above and its explanatory sentence
/// beneath, inside a grouped `Form` so the result is System Settings' own
/// chrome rather than a hand-built imitation of it.
///
/// **Nothing here composes a user-visible string from a number.** Every
/// caption — `Control.stepper`'s `valueLabel` above all — arrives finished
/// from Rust and is rendered verbatim. This is the exact site that shipped
/// "Check every 1 seconds" twice.
struct SchemaPane: View {
    let pane: SettingsPane
    let actions: SchemaPaneActions

    /// Free-text rows (a folder path) commit on Return or focus loss rather
    /// than on every keystroke — a path typed character by character would
    /// otherwise rewrite `config.toml` once per key, exactly the reasoning
    /// the previous window's directory field carried. Keyed by the row's own
    /// key so two text rows in one pane cannot share a draft.
    @State private var drafts: [String: String] = [:]
    @FocusState private var focusedField: String?

    var body: some View {
        Form {
            ForEach(Array(pane.groups.enumerated()), id: \.offset) { _, group in
                section(group)
            }

            // A destructive action stands alone at the foot of its pane
            // rather than sitting inline among the settings it would undo —
            // a button that discards work should never be one row away from
            // a switch someone is flicking.
            if !destructiveRows.isEmpty {
                Section {
                    ForEach(Array(destructiveRows.enumerated()), id: \.offset) { _, r in
                        rowView(r)
                    }
                }
            }

            // Offered only where something actually drifted from its factory
            // value: a Reset button that is always there says nothing, and
            // stops being read.
            if hasNonDefaultRow {
                Section {
                    HStack {
                        Spacer()
                        Button("Reset to defaults") { actions.reset(pane.id) }
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    // MARK: - Structure

    @ViewBuilder
    private func section(_ group: SettingGroup) -> some View {
        let rows = group.rows.filter { !isDestructiveAction($0) }
        if !rows.isEmpty {
            if let heading = group.heading {
                Section(heading) { rowList(rows) }
            } else {
                Section { rowList(rows) }
            }
        }
    }

    @ViewBuilder
    private func rowList(_ rows: [SettingRow]) -> some View {
        ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
            rowView(r)
        }
    }

    /// Label and control on one line, the row's grey explanatory sentence
    /// beneath it. The sentence is not decoration: a column of bare toggles
    /// reads as unfinished however many of them there are, and Rust has
    /// already written the words.
    @ViewBuilder
    private func rowView(_ r: SettingRow) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            control(for: r)
            if let help = r.help {
                Text(help)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        // A dependent row is indented under what governs it, and when that
        // thing is off it is *disabled, never hidden*: the user learns the
        // option exists and why it cannot be reached, and the pane does not
        // twitch as rows appear and vanish.
        .padding(.leading, r.indent ? 16 : 0)
        .disabled(!r.enabled || actions.additionallyDisabled(r))
    }

    // MARK: - One view per control

    @ViewBuilder
    private func control(for r: SettingRow) -> some View {
        switch r.control {
        case let .toggle(on):
            toggle(r, on: on)

        case let .stepper(value, minimum, maximum, step, valueLabel):
            LabeledContent(r.label) {
                // `valueLabel` is finished text from Rust — "30 seconds", "1
                // second". Never build one here.
                Stepper(value: intBinding(r, value), in: minimum...maximum, step: Int(step)) {
                    Text(valueLabel)
                        .monospacedDigit()
                }
            }

        case let .choice(selected, options):
            Picker(r.label, selection: choiceBinding(r, selected)) {
                ForEach(options, id: \.id) { option in
                    Text(option.label).tag(option.id)
                }
            }

        case let .intChoice(selected, options):
            // Sent back as an `Int`, never as a `Choice` spelled "30".
            Picker(r.label, selection: intChoiceBinding(r, selected)) {
                ForEach(options, id: \.value) { option in
                    Text(option.label).tag(option.value)
                }
            }

        case let .text(value, placeholder):
            LabeledContent(r.label) {
                TextField(placeholder, text: draftBinding(r, value))
                    .focused($focusedField, equals: fieldId(r))
                    .onSubmit { commitDraft(r, stored: value) }
                    .frame(minWidth: 200)
            }
            .onChange(of: focusedField) { _, now in
                if now != fieldId(r) { commitDraft(r, stored: value) }
            }

        case let .folder(value, resolvedLabel):
            LabeledContent(r.label) {
                HStack(spacing: 6) {
                    // The placeholder is Rust's own word for what happens
                    // when the field is left empty, not a Swift invention.
                    TextField(resolvedLabel, text: draftBinding(r, value))
                        .focused($focusedField, equals: fieldId(r))
                        .onSubmit { commitDraft(r, stored: value) }
                        .frame(minWidth: 200)
                    Button("Choose…") { chooseFolder(r, current: value) }
                    if !value.isEmpty {
                        Button("Automatic") { setDraft(r, "") }
                    }
                }
            }
            .onChange(of: focusedField) { _, now in
                if now != fieldId(r) { commitDraft(r, stored: value) }
            }

        case let .action(buttonLabel, destructive):
            HStack {
                Text(r.label)
                Spacer()
                Button(buttonLabel, role: Self.role(destructive)) {
                    actions.perform(r)
                }
            }

        case let .info(valueLabel):
            LabeledContent(r.label) {
                Text(valueLabel)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                    .textSelection(.enabled)
            }
        }
    }

    /// A toggle reads the stored value unless macOS holds the truth for it —
    /// the login item is registered (or not) whatever the file says, and a
    /// notification switch must never sit ON over a channel delivering
    /// nothing.
    @ViewBuilder
    private func toggle(_ r: SettingRow, on: Bool) -> some View {
        if let key = r.key, let system = actions.systemToggle(key) {
            Toggle(r.label, isOn: Binding(get: { system.isOn }, set: { system.set($0) }))
                .disabled(system.isDisabled)
        } else {
            Toggle(r.label, isOn: Binding(
                get: { on },
                set: { v in write(r, .bool(value: v)) }
            ))
        }
    }

    // MARK: - Bindings
    //
    // Every `get` below derives from `pane`, which is whatever the last
    // write returned — never from local state. A control that moves before
    // its write lands has nothing to move back to when the write fails, and
    // that is a defect already fixed once on this milestone.

    private func intBinding(_ r: SettingRow, _ value: Int64) -> Binding<Int64> {
        Binding(get: { value }, set: { v in write(r, .int(value: v)) })
    }

    private func intChoiceBinding(_ r: SettingRow, _ selected: Int64) -> Binding<Int64> {
        Binding(get: { selected }, set: { v in write(r, .int(value: v)) })
    }

    /// `PreferredTerminal` draws as a choice but stores **text**: its option
    /// ids are the names of applications Perch found on this machine, not the
    /// wire values of an enum Perch defines, and `set_setting` refuses an
    /// unrecognised `Choice` outright. Every other choice row sends `Choice`.
    /// Getting this one wrong makes the terminal picker fail silently.
    private func choiceBinding(_ r: SettingRow, _ selected: String) -> Binding<String> {
        Binding(
            get: { selected },
            set: { v in
                write(r, r.key == .preferredTerminal ? .text(value: v) : .choice(value: v))
            }
        )
    }

    private func draftBinding(_ r: SettingRow, _ stored: String) -> Binding<String> {
        Binding(
            get: { drafts[fieldId(r)] ?? stored },
            set: { drafts[fieldId(r)] = $0 }
        )
    }

    // MARK: - Text commits

    private func fieldId(_ r: SettingRow) -> String {
        r.key.map { String(describing: $0) } ?? r.label
    }

    private func commitDraft(_ r: SettingRow, stored: String) {
        guard let draft = drafts[fieldId(r)] else { return }
        drafts[fieldId(r)] = nil
        guard draft != stored else { return }
        write(r, .text(value: draft))
    }

    private func setDraft(_ r: SettingRow, _ value: String) {
        drafts[fieldId(r)] = nil
        write(r, .text(value: value))
    }

    private func chooseFolder(_ r: SettingRow, current: String) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        if !current.isEmpty {
            panel.directoryURL = URL(fileURLWithPath: current)
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        setDraft(r, url.path)
    }

    private func write(_ r: SettingRow, _ value: SettingValue) {
        guard let key = r.key else { return }
        actions.write(key, value)
    }

    // MARK: - Pane-level facts

    private var hasNonDefaultRow: Bool {
        pane.groups.contains { $0.rows.contains { $0.key != nil && !$0.isDefault } }
    }

    private var destructiveRows: [SettingRow] {
        pane.groups.flatMap { $0.rows }.filter(isDestructiveAction)
    }

    private func isDestructiveAction(_ r: SettingRow) -> Bool {
        if case let .action(_, destructive) = r.control { return destructive }
        return false
    }

    private static func role(_ destructive: Bool) -> ButtonRole? {
        destructive ? .destructive : nil
    }
}
