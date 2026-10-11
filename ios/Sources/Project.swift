import Combine
import SwiftUI

/// The open project and the chapter in the editor.
@MainActor
final class ProjectModel: ObservableObject {
    @Published var items: [Item] = []
    @Published var text = ""
    @Published var words = 0
    /// The chapter whose text is in `text`.
    @Published var editing: String?
    /// For the author; a tap removes it.
    @Published var notice = ""
    /// A sync is under way. The editor does not take text then: a merge may replace it.
    @Published var busy = false
    private var saved = ""
    @Published private(set) var loaded = false
    private var watch: Set<AnyCancellable> = []

    init() {
        // The model watches the app, not a view: a view under the editor does not see the change.
        let center = NotificationCenter.default
        center.publisher(for: UIApplication.didEnterBackgroundNotification).sink { [weak self] _ in
            // iOS gives about 30 s. What does not go up now goes at the next sync.
            let task = UIApplication.shared.beginBackgroundTask()
            Task {
                await self?.sync()
                UIApplication.shared.endBackgroundTask(task)
            }
        }.store(in: &watch)
        center.publisher(for: UIApplication.didBecomeActiveNotification).sink { [weak self] _ in
            Task { await self?.sync() }
        }.store(in: &watch)
    }

    private func run(_ work: () async throws -> Void) async {
        do { try await work() } catch { notice = error.localizedDescription }
    }

    /// Opens the project.
    func load(_ name: String) async {
        guard !loaded else { return await refresh() }
        await run {
            items = try await Core.open(name)
            loaded = true
        }
        await sync()
    }
    /// Reads the chapter list again, for the word counts.
    func refresh() async {
        await run { items = try await Core.tree() }
    }
    func add(_ title: String) async {
        await run {
            try await Core.addChapter(title)
            items = try await Core.tree()
        }
    }
    func rename(_ id: String, _ title: String) async {
        await run {
            try await Core.rename(id, title)
            items = try await Core.tree()
        }
    }

    func edit(_ item: Item) async {
        // The text of the chapter before this one is stored under its own id first.
        await save()
        await run {
            let stored = try await Core.read(item.id)
            text = stored
            saved = stored
            words = item.words
            editing = item.id
        }
    }
    func save() async {
        guard let id = editing, text != saved else { return }
        let writing = text
        await run {
            words = try await Core.write(id, writing)
            saved = writing
        }
    }
    func sync() async {
        guard loaded, !busy else { return }
        busy = true
        defer { busy = false }
        await save()
        await run {
            let round = try await Core.sync()
            if !round.message.isEmpty { notice = round.message }
            if round.changed {
                items = try await Core.tree()
                if let id = editing {
                    let stored = try await Core.read(id)
                    text = stored
                    saved = stored
                    words = items.first { $0.id == id }?.words ?? words
                }
            }
        }
    }
}

struct Notice: View {
    @ObservedObject var model: ProjectModel

    var body: some View {
        if !model.notice.isEmpty {
            Text(model.notice)
                .font(.footnote)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
                .foregroundStyle(Theme.warning)
                .background(Theme.surface)
                .onTapGesture { model.notice = "" }
        }
    }
}

struct ChaptersView: View {
    let name: String
    @StateObject private var model = ProjectModel()
    @State private var title = ""
    @State private var adding = false
    @State private var renaming: Item?
    @State private var shut: Set<String> = []

    var body: some View {
        List {
            Notice(model: model).listRowBackground(Theme.surface)
            if !model.loaded && model.notice.isEmpty {
                ProgressView().frame(maxWidth: .infinity).listRowBackground(Color.clear)
            }
            ForEach(["manuscript", "notes"], id: \.self) { section in
                // The sidebar style gives each heading an arrow that folds its section.
                Section(section == "manuscript" ? "Manuscript" : "Notes", isExpanded: Binding(get: { !shut.contains(section) }, set: { if $0 { shut.remove(section) } else { shut.insert(section) } })) {
                    ForEach(model.items.filter { $0.section == section }) { item in
                        NavigationLink(value: item) {
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(item.title)
                                    if let part = item.part {
                                        Text(part).font(.caption).foregroundStyle(Theme.muted)
                                    }
                                }
                                Spacer()
                                Text("\(item.words)").foregroundStyle(Theme.muted)
                            }
                        }
                        .swipeActions {
                            Button("Rename") {
                                title = item.title
                                renaming = item
                            }
                        }
                    }
                    .listRowBackground(Theme.surface)
                }
            }
        }
        .listStyle(.sidebar)
        .paper()
        .navigationTitle(name.replacingOccurrences(of: ".jw", with: ""))
        .navigationDestination(for: Item.self) { EditorView(item: $0, model: model) }
        .toolbar {
            Button {
                title = ""
                adding = true
            } label: {
                Image(systemName: "plus")
            }
        }
        .alert("New chapter", isPresented: $adding) {
            TextField("Title", text: $title)
            Button("Add") { Task { await model.add(title) } }
            Button("Cancel", role: .cancel) {}
        }
        .alert("Rename", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } }), presenting: renaming) { item in
            TextField("Title", text: $title)
            Button("Rename") { Task { await model.rename(item.id, title) } }
            Button("Cancel", role: .cancel) {}
        }
        .task { await model.load(name) }
        .refreshable { await model.sync() }
    }
}

/// The chapter as a reader sees it: markers gone, the text formatted.
enum Reading {
    enum Block: Hashable {
        case prose(String), quote(String), name(String), scene
    }

    /// One block for each line with text. The rules are those of src/core/prose.rs, made simple.
    static func blocks(_ text: String) -> [Block] {
        var quoted = false
        return text.split(whereSeparator: \.isNewline).compactMap { raw in
            var line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { return nil }
            if line.hasPrefix("## "), line.hasSuffix(" ##"), line.count > 6 {
                return .name(String(line.dropFirst(3).dropLast(3)))
            }
            let marks = Set(line.filter { !$0.isWhitespace })
            if line.lowercased() == "###newscene###" || (marks.count == 1 && "#*-_~=⁂".contains(marks.first!) && (marks.first == "#" || line.count >= 3 || marks.first == "⁂")) {
                quoted = false
                return .scene
            }
            if !quoted, line.hasPrefix(">") {
                quoted = true
                line = String(line.dropFirst())
            }
            guard quoted else { return .prose(line) }
            if line.hasSuffix("<") {
                quoted = false
                line = String(line.dropLast())
            }
            line = line.trimmingCharacters(in: .whitespaces)
            return line.isEmpty ? nil : .quote(line)
        }
    }

    /// `*italics*` and `**bold**` set, notes between `$$` in the muted colour.
    // ponytail: the system's Markdown reads the emphasis, so it also sets `_x_` and links. Use the rules of the library when that matters.
    static func line(_ text: String) -> AttributedString {
        var out = AttributedString()
        for (n, piece) in text.components(separatedBy: "$$").enumerated() {
            var part = (try? AttributedString(markdown: piece, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(piece)
            if n % 2 == 1 { part.foregroundColor = Theme.muted }
            out += part
        }
        return out
    }
}

struct ReadingView: View {
    let title: String
    let text: String

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(title).font(.largeTitle.bold()).foregroundStyle(Theme.muted).frame(maxWidth: .infinity).padding(.vertical, 12)
                ForEach(Array(Reading.blocks(text).enumerated()), id: \.offset) { _, block in
                    switch block {
                    case .prose(let line): Text(Reading.line(line))
                    case .quote(let line): Text(Reading.line(line)).italic().padding(.horizontal, 28)
                    case .name(let name): Text(name).font(.headline).foregroundStyle(Theme.muted).frame(maxWidth: .infinity)
                    case .scene: Text("* * *").foregroundStyle(Theme.muted).frame(maxWidth: .infinity).padding(.vertical, 6)
                    }
                }
            }
            .font(Theme.serif(18))
            .lineSpacing(10)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 18)
            .frame(maxWidth: 720)
            .frame(maxWidth: .infinity)
        }
    }
}

/// The text view of the editor, for the buttons of the toolbar.
final class Field {
    weak var view: UITextView?

    /// Puts `open` and `close` around the selection. With no selection the caret goes between them.
    func wrap(_ open: String, _ close: String) {
        guard let view, let range = view.selectedTextRange else { return }
        let picked = view.text(in: range) ?? ""
        view.replace(range, withText: open + picked + close)
        if let end = view.selectedTextRange?.end,
           let to = view.position(from: end, offset: -close.utf16.count),
           let from = view.position(from: to, offset: -picked.utf16.count) {
            view.selectedTextRange = view.textRange(from: from, to: to)
        }
    }
}

/// The field the author types in. UIKit: SwiftUI's field loses the caret when a button changes the text.
struct ProseEditor: UIViewRepresentable {
    @Binding var text: String
    let locked: Bool
    let field: Field

    final class Coordinator: NSObject, UITextViewDelegate, NSLayoutManagerDelegate {
        var text: Binding<String>
        init(_ text: Binding<String>) { self.text = text }
        func textViewDidChange(_ view: UITextView) { text.wrappedValue = view.text }
        func layoutManager(_ manager: NSLayoutManager, lineSpacingAfterGlyphAt glyph: Int, withProposedLineFragmentRect rect: CGRect) -> CGFloat { 10 }
    }
    func makeCoordinator() -> Coordinator { Coordinator($text) }

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.delegate = context.coordinator
        view.layoutManager.delegate = context.coordinator
        view.backgroundColor = .clear
        // Font size 18 and line height 1.6: the defaults of the desktop editor.
        view.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: UIFont(name: "LibertinusSerif-Regular", size: 18) ?? .systemFont(ofSize: 18))
        view.adjustsFontForContentSizeCategory = true
        view.textColor = UIColor(Theme.text)
        view.tintColor = UIColor(Theme.accent)
        view.textContainerInset = UIEdgeInsets(top: 8, left: 12, bottom: 24, right: 12)
        view.alwaysBounceVertical = true
        view.keyboardDismissMode = .interactive
        field.view = view
        func button(_ symbol: String, _ name: String, _ run: @escaping () -> Void) -> UIBarButtonItem {
            let item = UIBarButtonItem(image: UIImage(systemName: symbol), primaryAction: UIAction { _ in run() })
            item.accessibilityLabel = name
            return item
        }
        let bar = UIToolbar(frame: CGRect(x: 0, y: 0, width: 320, height: 44))
        bar.tintColor = view.tintColor
        bar.items = [
            button("italic", "Italic") { [field] in field.wrap("*", "*") },
            button("bold", "Bold") { [field] in field.wrap("**", "**") },
            button("note.text", "Note") { [field] in field.wrap("$$", "$$") },
            button("text.quote", "Quote") { [field] in field.wrap("> ", " <") },
            button("asterisk", "New scene") { [field] in field.wrap("\n\n###newscene###\n\n", "") },
            .flexibleSpace(),
            button("arrow.uturn.backward", "Undo") { [field] in field.view?.undoManager?.undo() },
            button("arrow.uturn.forward", "Redo") { [field] in field.view?.undoManager?.redo() },
            button("keyboard.chevron.compact.down", "Hide keyboard") { [field] in field.view?.resignFirstResponder() },
        ]
        view.inputAccessoryView = bar
        return view
    }
    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.text = $text
        // A sync replaced the text.
        if view.text != text { view.text = text }
        view.isEditable = !locked
    }
}

struct EditorView: View {
    let item: Item
    @ObservedObject var model: ProjectModel
    @State private var reading = false
    @State private var field = Field()

    var body: some View {
        Group {
            if model.editing == item.id, reading {
                ReadingView(title: item.title, text: model.text)
            } else if model.editing == item.id {
                // Width 720: the default of the desktop editor.
                ProseEditor(text: $model.text, locked: model.busy, field: field).frame(maxWidth: 720)
            } else {
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .safeAreaInset(edge: .top) { Notice(model: model) }
        .navigationTitle(item.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Text("\(model.words) words").font(.footnote).foregroundStyle(Theme.muted)
                Button(reading ? "Edit" : "Read", systemImage: reading ? "pencil" : "book") { reading.toggle() }
            }
        }
        .task { await model.edit(item) }
        // Stores the text when typing stops for two seconds.
        .task(id: model.text) {
            if (try? await Task.sleep(for: .seconds(2))) != nil {
                await model.save()
            }
        }
        .onDisappear {
            Task {
                await model.save()
                await model.refresh()
            }
        }
    }
}
