import Foundation

enum APIError: LocalizedError {
    case badURL
    case http(Int, String)

    var errorDescription: String? {
        switch self {
        case .badURL: return "The server address is not a valid URL."
        case .http(401, _): return "The server refused the token. Check it under Settings."
        case .http(let code, let msg): return msg.isEmpty ? "The server answered \(code)." : msg
        }
    }
}

/// The TellusHive server (`tellushive serve`) behind its HTTPS address.
final class API: @unchecked Sendable {
    let base: URL
    let token: String
    private let session: URLSession
    private let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    init(base: URL, token: String) {
        self.base = base
        self.token = token
        let cfg = URLSessionConfiguration.default
        // the server pings an open event stream every 15 s, so 90 s of silence means it is gone
        cfg.timeoutIntervalForRequest = 90
        cfg.timeoutIntervalForResource = 60 * 60 * 24
        cfg.waitsForConnectivity = true
        session = URLSession(configuration: cfg)
    }

    private func request(_ path: String, query: [URLQueryItem] = [], method: String = "GET", body: Data? = nil, contentType: String? = nil) throws -> URLRequest {
        guard var c = URLComponents(url: base.appendingPathComponent(path), resolvingAgainstBaseURL: false) else { throw APIError.badURL }
        if !query.isEmpty { c.queryItems = query }
        guard let url = c.url else { throw APIError.badURL }
        var r = URLRequest(url: url)
        r.httpMethod = method
        r.httpBody = body
        r.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        if let contentType { r.setValue(contentType, forHTTPHeaderField: "Content-Type") }
        return r
    }

    private func data(_ r: URLRequest) async throws -> Data {
        let (data, resp) = try await session.data(for: r)
        try check(resp, data)
        return data
    }

    private func check(_ resp: URLResponse, _ data: Data?) throws {
        guard let h = resp as? HTTPURLResponse else { return }
        guard (200..<300).contains(h.statusCode) else {
            let msg = data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }?["error"] as? String
            throw APIError.http(h.statusCode, msg ?? "")
        }
    }

    private static func botPath(_ id: String, _ tail: String) -> String {
        "api/bots/\(id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id)/\(tail)"
    }

    // MARK: endpoints

    func info() async throws -> [String: String] {
        try decoder.decode([String: String].self, from: try await data(request("api/info")))
    }

    func bots() async throws -> [BotInfo] {
        try decoder.decode([BotInfo].self, from: try await data(request("api/bots")))
    }

    func history(_ bot: String) async throws -> History {
        try decoder.decode(History.self, from: try await data(request(Self.botPath(bot, "history"))))
    }

    func avatar(_ bot: String) async throws -> Data {
        try await data(request(Self.botPath(bot, "avatar")))
    }

    func stop(_ bot: String) async throws {
        _ = try await data(request(Self.botPath(bot, "stop"), method: "POST"))
    }

    func newConversation(_ bot: String) async throws {
        _ = try await data(request(Self.botPath(bot, "new"), method: "POST"))
    }

    func upload(_ bot: String, name: String, data body: Data) async throws -> UploadResult {
        let r = try request(Self.botPath(bot, "upload"), query: [URLQueryItem(name: "name", value: name)], method: "POST", body: body, contentType: "application/octet-stream")
        return try decoder.decode(UploadResult.self, from: try await data(r))
    }

    func file(_ bot: String, path: String) async throws -> Data {
        try await data(request(Self.botPath(bot, "file"), query: [URLQueryItem(name: "path", value: path)]))
    }

    /// Downloads a project file to the caches folder (for Quick Look and sharing).
    func download(_ bot: String, file: FileRef) async throws -> URL {
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("files/\(bot)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(file.name)
        try await self.file(bot, path: file.path).write(to: url, options: .atomic)
        return url
    }

    /// Sends a message; the stream follows the task it starts.
    func send(_ bot: String, text: String, attachments: [String]) -> AsyncThrowingStream<LiveEvent, Error> {
        let body = try? JSONSerialization.data(withJSONObject: ["text": text, "attachments": attachments])
        return events { try self.request(Self.botPath(bot, "messages"), method: "POST", body: body, contentType: "application/json") }
    }

    /// Attaches to the running task of a bot (an `idle` event when nothing runs).
    func follow(_ bot: String) -> AsyncThrowingStream<LiveEvent, Error> {
        events { try self.request(Self.botPath(bot, "events")) }
    }

    private func events(_ make: @escaping @Sendable () throws -> URLRequest) -> AsyncThrowingStream<LiveEvent, Error> {
        AsyncThrowingStream { cont in
            let task = Task {
                do {
                    var r = try make()
                    r.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let (bytes, resp) = try await session.bytes(for: r)
                    if let h = resp as? HTTPURLResponse, !(200..<300).contains(h.statusCode) {
                        var body = Data()
                        for try await b in bytes { body.append(b) }
                        try check(resp, body)
                    }
                    for try await line in bytes.lines {
                        guard line.hasPrefix("data:") else { continue }
                        let json = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
                        if let e = try? decoder.decode(LiveEvent.self, from: Data(json.utf8)) {
                            cont.yield(e)
                        }
                    }
                    cont.finish()
                } catch {
                    cont.finish(throwing: error)
                }
            }
            cont.onTermination = { _ in task.cancel() }
        }
    }
}
