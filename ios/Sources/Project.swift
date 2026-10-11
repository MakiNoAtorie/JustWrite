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
    private var loaded = false

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
                .background(Color.yellow.opacity(0.3))
                .onTapGesture { model.notice = "" }
        }
    }
}

struct ChaptersView: View {
    let name: String
    @StateObject private var model = ProjectModel()
    @Environment(\.scenePhase) private var phase
    @State private var title = ""
    @State private var adding = false
    @State private var renaming: Item?

    var body: some View {
        List {
            Notice(model: model)
            ForEach(["manuscript", "notes"], id: \.self) { section in
                Section(section == "manuscript" ? "Manuscript" : "Notes") {
                    ForEach(model.items.filter { $0.section == section }) { item in
                        NavigationLink(value: item) {
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(item.title)
                                    if let part = item.part {
                                        Text(part).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                Text("\(item.words)").foregroundStyle(.secondary)
                            }
                        }
                        .swipeActions {
                            Button("Rename") {
                                title = item.title
                                renaming = item
                            }
                        }
                    }
                }
            }
        }
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
        .onChange(of: phase) { _, now in
            if now == .background {
                // iOS gives about 30 s. What does not go up now goes at the next sync.
                let task = UIApplication.shared.beginBackgroundTask()
                Task {
                    await model.sync()
                    UIApplication.shared.endBackgroundTask(task)
                }
            } else if now == .active {
                Task { await model.sync() }
            }
        }
    }
}

struct EditorView: View {
    let item: Item
    @ObservedObject var model: ProjectModel

    var body: some View {
        Group {
            if model.editing == item.id {
                TextEditor(text: $model.text).disabled(model.busy).padding(.horizontal, 12)
            } else {
                ProgressView()
            }
        }
        .safeAreaInset(edge: .top) { Notice(model: model) }
        .navigationTitle(item.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            Text("\(model.words) words").font(.footnote).foregroundStyle(.secondary)
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
