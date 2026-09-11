import AppKit
import SwiftUI
import PerchFFI

/// The editable model-price table, and — above it, where it cannot be missed
/// — every model this index has actually used that the table has no rate for.
///
/// **That second list is the point of this pane.** A price table shows what
/// it knows; it cannot show what it is missing, and a model with no row
/// contributes every one of its tokens and none of its dollars, which reads
/// everywhere else in the app as work that was free. Perch makes no network
/// requests and will not guess a rate, so the only honest fix is the user
/// typing one — and the one thing that must not stand between them and that
/// is retyping a model id from memory and silently mistyping it. Each
/// unpriced model therefore carries its own button, and the id arrives
/// already filled in.
///
/// Nothing here formats a number for display. The four rates arrive from
/// Rust as the text they should appear as, go back as the text the user
/// typed, and become an `f64` exactly once, in `ui::prices::parse_rates` —
/// which is also where the sentence explaining a rate it will not accept is
/// written.
struct PricesPane: View {
    let engine: PerchEngine
    /// Bumped each time the settings window is (re)presented, so reopening
    /// re-reads the table rather than showing the snapshot from last time.
    let refreshToken: Int
    /// Called after any write that lands, so the sidebar's "3 unpriced"
    /// badge and this pane's own preview stop disagreeing with the table the
    /// moment it changes.
    let schemaChanged: @MainActor () -> Void

    @State private var model: PricesModel?
    @State private var loadError: String?
    /// A write that failed. Rendered against the row it belongs to where
    /// there is one, so "Output is “seventy five”, which is not a number"
    /// appears beside the box holding it.
    @State private var rowError: [String: String] = [:]
    /// A failure with no row to sit beside — a reset that could not run, a
    /// new model that could not be added.
    @State private var paneError: String?

    /// In-flight edits, keyed by model. A row is only ever *drawn* from
    /// `model`; a draft exists from the first keystroke until the write
    /// lands, and is dropped afterwards so the row goes back to reading
    /// whatever Rust returned. Nothing is mutated optimistically.
    @State private var drafts: [String: RateFields] = [:]
    @FocusState private var focused: String?

    @State private var adding = false
    @State private var newModel = ""
    @State private var newRates = RateFields(input: "", output: "", cacheRead: "", cacheWrite: "")
    @State private var confirmingReset = false
    @State private var removing: String?

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
    private func content(_ m: PricesModel) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let paneError {
                    banner(paneError, color: .red)
                }

                if let summary = m.unpricedSummary {
                    unpricedSection(m.unpriced, summary: summary)
                }

                table(m)

                if adding {
                    addForm()
                }

                HStack(spacing: 10) {
                    Button("Add a model…") { beginAdd(model: "") }
                        .disabled(adding)
                    Spacer()
                    Button("Reset to built-in prices…", role: .destructive) {
                        confirmingReset = true
                    }
                }

                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(m.notes.enumerated()), id: \.offset) { _, note in
                        Text(note)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .confirmationDialog(
            // The warning itself is Rust's; this is the verb on the button.
            "Reset the price table?",
            isPresented: $confirmingReset,
            titleVisibility: .visible
        ) {
            Button("Reset prices", role: .destructive) { reset() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(m.resetWarning)
        }
    }

    // MARK: - The gap the table cannot show

    /// Drawn above the table, not below it: this is the thing a user came
    /// here to fix, whether or not they knew it when they arrived.
    @ViewBuilder
    private func unpricedSection(_ models: [UnpricedModel], summary: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label {
                Text(summary)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(Color.perchWaiting)
            }

            VStack(spacing: 0) {
                ForEach(Array(models.enumerated()), id: \.offset) { index, unpriced in
                    if index > 0 { Divider() }
                    HStack(spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(unpriced.model)
                                .font(.callout.monospaced())
                                .textSelection(.enabled)
                            Text(unpriced.tokensLabel)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 8)
                        // The whole affordance: the id is carried across, so
                        // adding a rate for a model already in use is one
                        // click and four numbers, with nothing to mistype.
                        Button("Add a price…") { beginAdd(model: unpriced.model) }
                    }
                    .padding(.vertical, 7)
                    .padding(.horizontal, 10)
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

    // MARK: - The table

    @ViewBuilder
    private func table(_ m: PricesModel) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            // The sentence that stops a millionfold error. Rust's, and above
            // the columns rather than tucked underneath them.
            Text(m.header)
                .font(.callout.weight(.medium))

            VStack(spacing: 0) {
                headerRow
                ForEach(Array(m.rows.enumerated()), id: \.offset) { _, row in
                    Divider()
                    priceRow(row)
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(Color.primary.opacity(0.10))
            )
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }

    private var headerRow: some View {
        HStack(spacing: 8) {
            Text("Model").frame(width: modelWidth, alignment: .leading)
            ForEach(Self.columnTitles, id: \.self) { title in
                Text(title).frame(width: rateWidth, alignment: .leading)
            }
            Spacer(minLength: 0)
        }
        .font(.caption2.weight(.semibold))
        .foregroundStyle(.secondary)
        .textCase(.uppercase)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor))
    }

    @ViewBuilder
    private func priceRow(_ row: ModelPriceRow) -> some View {
        let rates = drafts[row.model] ?? row.rates
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(row.model)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                    // Finished in Rust; `nil` for a model never used, which
                    // is a different thing from one that used nothing.
                    if let usage = row.usageLabel {
                        Text(usage)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(width: modelWidth, alignment: .leading)

                rateField(row, keyPath: \.input, value: rates.input)
                rateField(row, keyPath: \.output, value: rates.output)
                rateField(row, keyPath: \.cacheRead, value: rates.cacheRead)
                rateField(row, keyPath: \.cacheWrite, value: rates.cacheWrite)

                Spacer(minLength: 0)

                Button {
                    removing = row.model
                } label: {
                    Image(systemName: "minus.circle")
                }
                .buttonStyle(.borderless)
                .help("Forget this rate. The model's tokens keep counting and add no cost.")
                .accessibilityLabel("Remove price for \(row.model)")
            }

            // The refusal, against the row it refused.
            if let message = rowError[row.model] {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .confirmationDialog(
            "Remove this price?",
            isPresented: Binding(
                get: { removing == row.model },
                set: { if !$0 { removing = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove price", role: .destructive) { remove(row.model) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Its turns keep counting their tokens and stop contributing any cost, "
                + "until you give it a rate again."
            )
        }
    }

    /// One editable rate. Commits on Return or when focus leaves — a rate
    /// rewritten on every keystroke would write "1", "1.", "1.5" to the
    /// database in turn, and the first two of those are real rates that
    /// would briefly be in force.
    private func rateField(
        _ row: ModelPriceRow,
        keyPath: WritableKeyPath<RateFields, String>,
        value: String
    ) -> some View {
        let id = "\(row.model)|\(String(describing: keyPath))"
        return TextField(
            "",
            text: Binding(
                get: { value },
                set: { typed in
                    var draft = drafts[row.model] ?? row.rates
                    draft[keyPath: keyPath] = typed
                    drafts[row.model] = draft
                }
            )
        )
        .textFieldStyle(.roundedBorder)
        .multilineTextAlignment(.trailing)
        .frame(width: rateWidth)
        .focused($focused, equals: id)
        .onSubmit { commit(row) }
        .onChange(of: focused) { _, now in
            if now != id { commit(row) }
        }
    }

    // MARK: - Adding a model

    @ViewBuilder
    private func addForm() -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("Model")
                    .frame(width: 60, alignment: .leading)
                TextField("claude-…", text: $newModel)
                    .textFieldStyle(.roundedBorder)
                    .font(.callout.monospaced())
                    .frame(maxWidth: 320)
            }

            HStack(spacing: 8) {
                Text("Rates")
                    .frame(width: 60, alignment: .leading)
                newRateField("Input", \.input)
                newRateField("Output", \.output)
                newRateField("Cache read", \.cacheRead)
                newRateField("Cache write", \.cacheWrite)
            }

            // Perch will not invent a rate; a user may choose to start from
            // one they already have, see the four numbers land in the boxes,
            // and change them. That is the user supplying the number, which
            // is the only way a number gets in here.
            if let sources = model?.rows, !sources.isEmpty {
                HStack(spacing: 8) {
                    Text("")
                        .frame(width: 60)
                    Menu("Copy rates from…") {
                        ForEach(Array(sources.enumerated()), id: \.offset) { _, source in
                            Button(source.model) { newRates = source.rates }
                        }
                    }
                    .frame(width: 180)
                    Text("Fills the four boxes so you can check them and change what differs.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            HStack {
                Spacer()
                Button("Cancel") { cancelAdd() }
                Button("Add") { add() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(newModel.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color(nsColor: .controlBackgroundColor))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.10))
        )
    }

    private func newRateField(
        _ title: String,
        _ keyPath: WritableKeyPath<RateFields, String>
    ) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            TextField(
                "",
                text: Binding(
                    get: { newRates[keyPath: keyPath] },
                    set: { newRates[keyPath: keyPath] = $0 }
                )
            )
            .textFieldStyle(.roundedBorder)
            .multilineTextAlignment(.trailing)
            .frame(width: rateWidth)
        }
    }

    private func beginAdd(model: String) {
        paneError = nil
        newModel = model
        newRates = RateFields(input: "", output: "", cacheRead: "", cacheWrite: "")
        adding = true
    }

    private func cancelAdd() {
        adding = false
        newModel = ""
        paneError = nil
    }

    // MARK: - Writes
    //
    // Every one of these adopts the pane the call returned. Nothing is
    // changed locally first, so a write that fails leaves the table showing
    // exactly what is stored — which is the whole point: a rate that could
    // not be parsed must not appear to have been accepted.

    private func commit(_ row: ModelPriceRow) {
        guard let draft = drafts[row.model] else { return }
        guard draft != row.rates else {
            drafts[row.model] = nil
            return
        }
        Task {
            switch await engine.setPrice(model: row.model, rates: draft) {
            case .success(let m):
                drafts[row.model] = nil
                rowError[row.model] = nil
                adopt(m)
            case .failure(let error):
                // The draft stays put so the user can see and fix what they
                // typed; the stored rate is untouched either way.
                rowError[row.model] = error.localizedDescription
            }
        }
    }

    private func add() {
        Task {
            switch await engine.setPrice(model: newModel, rates: newRates) {
            case .success(let m):
                cancelAdd()
                adopt(m)
            case .failure(let error):
                paneError = error.localizedDescription
            }
        }
    }

    private func remove(_ model: String) {
        Task {
            switch await engine.removePrice(model: model) {
            case .success(let m):
                drafts[model] = nil
                rowError[model] = nil
                adopt(m)
            case .failure(let error):
                paneError = error.localizedDescription
            }
        }
    }

    private func reset() {
        Task {
            switch await engine.resetPrices() {
            case .success(let m):
                drafts = [:]
                rowError = [:]
                adopt(m)
            case .failure(let error):
                // Reset is one transaction in Rust: a failure changed
                // nothing, and saying so is the difference between a table
                // the user can trust and one they cannot.
                paneError = error.localizedDescription
            }
        }
    }

    private func adopt(_ m: PricesModel) {
        model = m
        paneError = nil
        loadError = nil
        schemaChanged()
    }

    private func load() async {
        rowError = [:]
        paneError = nil
        switch await engine.prices() {
        case .success(let m):
            model = m
            loadError = nil
        case .failure(let error):
            loadError = error.localizedDescription
        }
    }

    // MARK: - Chrome

    private func banner(_ text: String, color: Color) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(color)
            .fixedSize(horizontal: false, vertical: true)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(color.opacity(0.12))
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
    }

    private static let columnTitles = ["Input", "Output", "Cache read", "Cache write"]
    private var modelWidth: CGFloat { 220 }
    private var rateWidth: CGFloat { 84 }
}
