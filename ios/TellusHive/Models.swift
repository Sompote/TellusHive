import Foundation
import SwiftUI

/// A bot as `GET /api/bots` lists it.
struct BotInfo: Codable, Identifiable, Hashable {
    let id: String
    let name: String
    let tagline: String
    let color: String
    let team: String
    let hasAvatar: Bool
    let busy: Bool
}

/// A file in the bot's project (a calculation, drawing, report, picture or attachment).
struct FileRef: Codable, Hashable {
    let kind: String
    let path: String
    let name: String

    var ext: String { (name as NSString).pathExtension.lowercased() }
    var isImage: Bool { ["png", "jpg", "jpeg", "gif", "webp", "heic"].contains(ext) }
    var symbol: String {
        switch ext {
        case "pdf": return "doc.richtext"
        case "png", "jpg", "jpeg", "gif", "webp", "heic", "svg": return "photo"
        case "dxf", "dwg": return "pencil.and.ruler"
        case "xlsx", "csv": return "tablecells"
        case "docx", "md", "txt", "html": return "doc.text"
        default: return "doc"
        }
    }
}

/// One entry of `GET /api/bots/{id}/history`.
struct HistoryEntry: Decodable {
    let id: Int
    let kind: String
    let text: String?
    let agent: String?
    let files: [FileRef]?
    let file: FileRef?
    let outcome: String?
    let seconds: Int?
    let tokens: Int?
}

struct History: Decodable {
    let entries: [HistoryEntry]
    let busy: Bool
    let activity: String
}

/// One Server-Sent Event of a running task.
struct LiveEvent: Decodable {
    let type: String
    let text: String?
    let agent: String?
    let file: FileRef?
    let outcome: String?
    let error: String?
    let activity: String?
    let partial: String?
}

struct UploadResult: Decodable {
    let path: String
    let name: String
}

/// A row of the chat as the phone draws it.
struct ChatItem: Identifiable, Equatable {
    enum Role: Equatable { case user, bot, team, note, file, error, end }
    let id: String
    let role: Role
    var text: String = ""
    var agent: String = ""
    var files: [FileRef] = []

    static func from(_ e: HistoryEntry) -> ChatItem? {
        let id = "h\(e.id)"
        switch e.kind {
        case "user": return ChatItem(id: id, role: .user, text: e.text ?? "", files: e.files ?? [])
        case "bot": return ChatItem(id: id, role: .bot, text: e.text ?? "", agent: e.agent ?? "")
        case "team": return ChatItem(id: id, role: .team, text: e.text ?? "", agent: e.agent ?? "")
        case "note": return ChatItem(id: id, role: .note, text: e.text ?? "")
        case "file": return e.file.map { ChatItem(id: id, role: .file, files: [$0]) }
        case "error": return ChatItem(id: id, role: .error, text: e.text ?? "")
        case "end":
            let outcome = e.outcome ?? ""
            // a finished task needs no marker; stopped, failed and interrupted ones do
            guard outcome != "finished" else { return nil }
            return ChatItem(id: id, role: .end, text: outcome.capitalized)
        default: return nil
        }
    }
}

extension Color {
    /// "#rrggbb" from the bot's settings.
    init(hex: String, fallback: Color = .accentColor) {
        let s = hex.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "#", with: "")
        guard s.count == 6, let v = UInt32(s, radix: 16) else {
            self = fallback
            return
        }
        self = Color(red: Double((v >> 16) & 0xff) / 255, green: Double((v >> 8) & 0xff) / 255, blue: Double(v & 0xff) / 255)
    }
}
