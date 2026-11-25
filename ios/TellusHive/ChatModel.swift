import Foundation
import Observation

/// One bot's conversation on the phone: the transcript from the server, the reply being
/// written, and what the team is doing now. A task runs on the server, so leaving the app
/// does not stop it; coming back reloads the transcript and follows the task again.
@Observable
@MainActor
final class ChatModel {
    let bot: BotInfo
    private let api: API

    var items: [ChatItem] = []
    /// The reply as the bot writes it (shown as a growing bubble).
    var streaming = ""
    var busy = false
    var activity = ""
    var error: String?
    var loaded = false
    /// Files uploaded for the next message.
    var pending: [UploadResult] = []
    var uploading = 0

    private var follow: Task<Void, Never>?
    private var liveCount = 0

    init(bot: BotInfo, api: API) {
        self.bot = bot
        self.api = api
    }

    func load() async {
        do {
            let h = try await api.history(bot.id)
            items = h.entries.compactMap(ChatItem.from)
            busy = h.busy
            activity = h.activity
            error = nil
            loaded = true
            if h.busy, follow == nil {
                run(api.follow(bot.id))
            }
        } catch {
            if !Task.isCancelled { self.error = error.localizedDescription }
        }
    }

    var canSend: Bool { !busy && uploading == 0 }

    func send(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard canSend, !text.isEmpty || !pending.isEmpty else { return }
        let files = pending
        pending = []
        items.append(ChatItem(id: live(), role: .user, text: text, files: files.map { FileRef(kind: "Attachments", path: $0.path, name: $0.name) }))
        busy = true
        activity = "Sending"
        Haptics.tap()
        run(api.send(bot.id, text: text, attachments: files.map(\.path)))
    }

    func stop() {
        Task { try? await api.stop(bot.id) }
        activity = "Stopping"
    }

    func newConversation() async {
        do {
            try await api.newConversation(bot.id)
            items = []
            streaming = ""
        } catch {
            self.error = error.localizedDescription
        }
    }

    func attach(name: String, data: Data) {
        uploading += 1
        Task {
            defer { uploading -= 1 }
            do {
                pending.append(try await api.upload(bot.id, name: name, data: data))
            } catch {
                self.error = "Upload of \(name) failed: \(error.localizedDescription)"
            }
        }
    }

    func removePending(_ u: UploadResult) {
        pending.removeAll { $0.path == u.path }
    }

    /// Stops following (the view went away); the task goes on on the server.
    func detach() {
        follow?.cancel()
        follow = nil
    }

    private func live() -> String {
        liveCount += 1
        return "live\(liveCount)"
    }

    private func run(_ stream: AsyncThrowingStream<LiveEvent, Error>) {
        follow?.cancel()
        follow = Task { [weak self] in
            do {
                for try await e in stream {
                    guard let self else { return }
                    self.apply(e)
                }
            } catch {
                guard let self, !Task.isCancelled else { return }
                // the connection dropped (network change, app suspended): the task goes on
                // on the server; the transcript is reloaded when the app is back
                self.activity = "Reconnecting"
                try? await Task.sleep(for: .seconds(2))
                self.follow = nil
                await self.load()
                return
            }
            guard let self else { return }
            self.follow = nil
        }
    }

    private func apply(_ e: LiveEvent) {
        switch e.type {
        case "started":
            busy = true
        case "state":
            busy = true
            activity = e.activity ?? ""
            streaming = e.partial ?? ""
        case "delta":
            streaming += e.text ?? ""
        case "message":
            streaming = ""
            items.append(ChatItem(id: live(), role: .bot, text: e.text ?? "", agent: e.agent ?? ""))
        case "team":
            items.append(ChatItem(id: live(), role: .team, text: e.text ?? "", agent: e.agent ?? ""))
        case "note":
            items.append(ChatItem(id: live(), role: .note, text: e.text ?? ""))
        case "file":
            if let f = e.file { items.append(ChatItem(id: live(), role: .file, files: [f])) }
        case "error":
            items.append(ChatItem(id: live(), role: .error, text: e.text ?? ""))
        case "activity":
            activity = e.text ?? ""
        case "done":
            busy = false
            streaming = ""
            activity = ""
            if e.outcome == "finished" { Haptics.success() } else { Haptics.warning() }
            // the server's transcript is the record: take it, with its ids
            Task { await load() }
        case "idle":
            busy = false
            streaming = ""
            activity = ""
        default:
            break
        }
    }
}
