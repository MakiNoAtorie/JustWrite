import SwiftUI

@main
struct JustWriteApp: App {
    @AppStorage("connected") private var connected = false

    init() {
        // The library names cloud lines and conflict files in local time and reads the zone here.
        setenv("TZ", TimeZone.current.identifier, 1)
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if connected { ProjectsView() } else { ConnectView() }
            }
        }
    }
}

struct ConnectView: View {
    @AppStorage("connected") private var connected = false
    @AppStorage("server") private var server = ""
    @AppStorage("user") private var user = ""
    @State private var password = ""
    @State private var note = ""
    @State private var busy = false

    var body: some View {
        Form {
            Section {
                TextField("Server (https://…)", text: $server).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                TextField("User", text: $user).textInputAutocapitalization(.never).autocorrectionDisabled()
                SecureField("App password", text: $password)
            } footer: {
                Text("Make an app password in the Nextcloud web settings: Security, Devices & sessions.")
            }
            Button(busy ? "Connecting…" : "Connect") {
                busy = true
                Task { @MainActor in
                    do {
                        _ = try await Core.connect(server: server, user: user, password: password)
                        connected = true
                    } catch {
                        note = error.localizedDescription
                    }
                    busy = false
                }
            }
            .disabled(busy || server.isEmpty || user.isEmpty || password.isEmpty)
            if !note.isEmpty {
                Text(note).foregroundStyle(.red)
            }
        }
        .navigationTitle("Nextcloud")
    }
}

struct ProjectsView: View {
    @AppStorage("connected") private var connected = false
    @State private var names: [String] = []
    @State private var note = ""

    var body: some View {
        List {
            if !note.isEmpty {
                Text(note).font(.footnote).foregroundStyle(.red)
            }
            ForEach(names, id: \.self) { name in
                NavigationLink(name.replacingOccurrences(of: ".jw", with: ""), value: name)
            }
        }
        .navigationTitle("Projects")
        .navigationDestination(for: String.self) { ChaptersView(name: $0) }
        .toolbar {
            Button("Connection") { connected = false }
        }
        .task { await load() }
        .refreshable { await load() }
    }

    @MainActor
    private func load() async {
        do {
            names = try await Core.projects()
            note = ""
        } catch {
            note = error.localizedDescription
        }
    }
}
