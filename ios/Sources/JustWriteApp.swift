import CoreText
import SwiftUI

/// The colours and the editor font of the desktop app (src/render/theme.rs), light or dark with the system.
enum Theme {
    private static func pair(_ light: UInt32, _ dark: UInt32) -> Color {
        func colour(_ hex: UInt32) -> UIColor {
            UIColor(red: CGFloat(hex >> 16 & 255) / 255, green: CGFloat(hex >> 8 & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: 1)
        }
        return Color(UIColor { $0.userInterfaceStyle == .dark ? colour(dark) : colour(light) })
    }
    static let background = pair(0xfaf8f4, 0x211e1b)
    static let surface = pair(0xf1ede6, 0x29241f)
    static let text = pair(0x302b26, 0xe9e2d6)
    static let muted = pair(0x6e6355, 0xa3988b)
    /// The accent #d9aa72 as a text colour: `accent_text` of the desktop.
    static let accent = pair(0x6b5741, 0xdebb90)
    static let danger = pair(0xb8433f, 0xe8837f)
    static let warning = pair(0x9a5a1f, 0xe0a070)

    static func serif(_ size: CGFloat) -> Font { .custom("LibertinusSerif-Regular", size: size) }
}

extension View {
    /// A list or form on the app's background.
    func paper() -> some View {
        scrollContentBackground(.hidden).background(Theme.background).foregroundStyle(Theme.text)
    }
}

@main
struct JustWriteApp: App {
    @AppStorage("connected") private var connected = false

    init() {
        // The library names cloud lines and conflict files in local time and reads the zone here.
        // It finds no zone data on iOS, so the zone goes as an offset, west of Greenwich positive.
        // ponytail: a change to or from summer time is seen at the next launch.
        let west = -TimeZone.current.secondsFromGMT()
        setenv("TZ", String(format: "LCL%@%d:%02d", west < 0 ? "-" : "", abs(west) / 3600, abs(west) % 3600 / 60), 1)
        for font in Bundle.main.urls(forResourcesWithExtension: "otf", subdirectory: nil) ?? [] {
            CTFontManagerRegisterFontsForURL(font as CFURL, .process, nil)
        }
        assert(Reading.blocks("a\n\n###newscene###\n## Two ##\n> q\nby <\nb") == [.prose("a"), .scene, .name("Two"), .quote("q"), .quote("by"), .prose("b")])
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if connected { ProjectsView() } else { ConnectView() }
            }
            .tint(Theme.accent)
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
                Text("Make an app password in the Nextcloud web settings: Security, Devices & sessions.").foregroundStyle(Theme.muted)
            }
            .listRowBackground(Theme.surface)
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
            .listRowBackground(Theme.surface)
            if !note.isEmpty {
                Text(note).foregroundStyle(Theme.danger).listRowBackground(Theme.surface)
            }
        }
        .paper()
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
                Text(note).font(.footnote).foregroundStyle(Theme.danger).listRowBackground(Theme.surface)
            }
            ForEach(names, id: \.self) { name in
                NavigationLink(name.replacingOccurrences(of: ".jw", with: ""), value: name)
            }
            .listRowBackground(Theme.surface)
        }
        .paper()
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
