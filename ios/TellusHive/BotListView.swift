import SwiftUI

/// The bots on the server, like a messenger's chat list.
@MainActor
struct BotListView: View {
    let api: API
    @Environment(\.scenePhase) private var phase
    @State private var bots: [BotInfo] = []
    @State private var error: String?
    @State private var loading = true
    @State private var settings = false

    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Label(error, systemImage: "wifi.exclamationmark")
                        .foregroundStyle(.red)
                }
                ForEach(bots) { bot in
                    NavigationLink(value: bot) {
                        BotRow(bot: bot, api: api)
                    }
                }
            }
            .overlay {
                if loading && bots.isEmpty {
                    ProgressView()
                } else if bots.isEmpty && error == nil {
                    ContentUnavailableView("No bots yet", systemImage: "person.2", description: Text("Make a bot in the Bots workspace of TellusHive on the server; it appears here."))
                }
            }
            .navigationTitle("Bots")
            .navigationDestination(for: BotInfo.self) { bot in
                ChatScreen(model: ChatModel(bot: bot, api: api), api: api)
            }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { settings = true } label: { Image(systemName: "gearshape") }
                        .accessibilityLabel("Server settings")
                }
            }
            .sheet(isPresented: $settings) {
                NavigationStack { ConnectView() }
            }
            .refreshable { await load() }
            .task { await load() }
            .onChange(of: phase) { _, p in
                if p == .active { Task { await load() } }
            }
        }
    }

    private func load() async {
        do {
            bots = try await api.bots()
            error = nil
        } catch {
            if !Task.isCancelled { self.error = error.localizedDescription }
        }
        loading = false
    }
}

struct BotRow: View {
    let bot: BotInfo
    let api: API

    var body: some View {
        HStack(spacing: 12) {
            Avatar(bot: bot, api: api, size: 48)
            VStack(alignment: .leading, spacing: 2) {
                Text(bot.name).font(.headline)
                Text(bot.tagline.isEmpty ? bot.team : bot.tagline)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            if bot.busy {
                Label("Working", systemImage: "circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.green)
                    .font(.caption2)
                    .accessibilityLabel("Working")
            }
        }
        .padding(.vertical, 4)
    }
}

/// The bot's picture, or its initial on its colour.
@MainActor
struct Avatar: View {
    let bot: BotInfo
    let api: API
    var size: CGFloat = 36
    @State private var image: PlatformImage?

    var body: some View {
        ZStack {
            if let image {
                Image(platform: image).resizable().scaledToFill()
            } else {
                Circle().fill(Color(hex: bot.color).gradient)
                Text(String(bot.name.prefix(1)).uppercased())
                    .font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundStyle(.white)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .task(id: bot.id) {
            guard bot.hasAvatar, image == nil else { return }
            if let cached = AvatarCache.shared.object(forKey: bot.id as NSString) {
                image = cached
                return
            }
            if let d = try? await api.avatar(bot.id), let img = PlatformImage(data: d) {
                AvatarCache.shared.setObject(img, forKey: bot.id as NSString)
                image = img
            }
        }
        .accessibilityHidden(true)
    }
}

enum AvatarCache {
    static let shared = NSCache<NSString, PlatformImage>()
}
