import Foundation

struct CoreError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// A chapter or a notes page.
struct Item: Decodable, Identifiable, Hashable {
    let id: String
    let title: String
    let section: String
    let part: String?
    let words: Int
}

/// What a sync did.
struct Round: Decodable {
    let changed: Bool
    let message: String
}

private struct Answer<T: Decodable>: Decodable {
    let ok: T?
    let error: String?
}

/// The Rust library. Calls run one at a time and off the main thread: a sync can take long.
enum Core {
    private static let queue = DispatchQueue(label: "justwrite.core")
    private static let data: String = {
        let folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder.path
    }()

    private static func call<T: Decodable>(_ function: @escaping () -> UnsafeMutablePointer<CChar>?) async throws -> T? {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T?, Error>) in
            queue.async {
                let result: Result<T?, Error> = Result {
                    guard let reply = function() else { throw CoreError(message: "The library gave no answer") }
                    defer { jw_free(reply) }
                    let answer = try JSONDecoder().decode(Answer<T>.self, from: Data(String(cString: reply).utf8))
                    if let error = answer.error { throw CoreError(message: error) }
                    return answer.ok
                }
                continuation.resume(with: result)
            }
        }
    }
    private static func value<T: Decodable>(_ function: @escaping () -> UnsafeMutablePointer<CChar>?) async throws -> T {
        let answer: T? = try await call(function)
        guard let answer else { throw CoreError(message: "The library gave an empty answer") }
        return answer
    }

    static func connect(server: String, user: String, password: String) async throws -> String {
        try await value { jw_connect(data, server, user, password) }
    }
    static func projects() async throws -> [String] {
        try await value { jw_projects(data) }
    }
    /// Opens a project and gives its chapters. The project that was open is closed.
    static func open(_ name: String) async throws -> [Item] {
        try await value { jw_open(data, name) }
    }
    static func tree() async throws -> [Item] {
        try await value { jw_tree() }
    }
    static func read(_ id: String) async throws -> String {
        try await value { jw_read(id) }
    }
    /// Gives the word count.
    static func write(_ id: String, _ text: String) async throws -> Int {
        try await value { jw_write(id, text) }
    }
    static func addChapter(_ title: String) async throws {
        let _: String = try await value { jw_add_chapter(title) }
    }
    static func rename(_ id: String, _ title: String) async throws {
        let _: Bool? = try await call { jw_rename(id, title) }
    }
    static func sync() async throws -> Round {
        try await value { jw_sync() }
    }
}
