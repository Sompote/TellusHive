import PhotosUI
import QuickLook
import SwiftUI
import UniformTypeIdentifiers

/// The conversation with one bot.
@MainActor
struct ChatScreen: View {
    @State private var model: ChatModel
    let api: API
    @Environment(\.scenePhase) private var phase
    @State private var input = ""
    @State private var photos: [PhotosPickerItem] = []
    @State private var pickingPhotos = false
    @State private var pickingFiles = false
    @State private var camera = false
    @State private var preview: URL?
    @State private var confirmNew = false
    @FocusState private var focused: Bool

    init(model: ChatModel, api: API) {
        _model = State(initialValue: model)
        self.api = api
    }

    var body: some View {
        pickers(chrome(transcript))
            .task { await model.load() }
            .onChange(of: phase) { _, p in
                if p == .active { Task { await model.load() } }
            }
            .onDisappear { model.detach() }
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 10) {
                    ForEach(model.items) { item in
                        MessageRow(item: item, bot: model.bot, api: api, open: open)
                            .id(item.id)
                    }
                    if !model.streaming.isEmpty {
                        MessageRow(item: ChatItem(id: "streaming", role: .bot, text: model.streaming), bot: model.bot, api: api, open: open)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: model.items.count) { _, _ in
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            .onChange(of: model.streaming) { _, _ in
                proxy.scrollTo("bottom", anchor: .bottom)
            }
            .overlay {
                if !model.loaded && model.error == nil { ProgressView() }
            }
        }
    }

    private func chrome(_ v: some View) -> some View {
        v.safeAreaInset(edge: .bottom, spacing: 0) { composer }
        .navigationTitle(model.bot.name)
        .inlineTitle()
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("New conversation", systemImage: "square.and.pencil") { confirmNew = true }
                        .disabled(model.busy)
                    Button("Reload", systemImage: "arrow.clockwise") { Task { await model.load() } }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
        .confirmationDialog("Start a new conversation with \(model.bot.name)?", isPresented: $confirmNew, titleVisibility: .visible) {
            Button("New conversation", role: .destructive) { Task { await model.newConversation() } }
        } message: {
            Text("The bot forgets this conversation. Its files stay in the project.")
        }
        .alert("Problem", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) {
            Button("OK") { model.error = nil }
        } message: {
            Text(model.error ?? "")
        }
    }

    private func pickers(_ v: some View) -> some View {
        v.photosPicker(isPresented: $pickingPhotos, selection: $photos, maxSelectionCount: 6, matching: .images)
        .onChange(of: photos) { _, items in
            for (i, item) in items.enumerated() {
                Task {
                    if let d = try? await item.loadTransferable(type: Data.self) {
                        model.attach(name: "photo-\(Int(Date().timeIntervalSince1970))-\(i + 1).jpg", data: jpegForUpload(d))
                    }
                }
            }
            photos = []
        }
        .fileImporter(isPresented: $pickingFiles, allowedContentTypes: [.pdf, .image, .plainText, .commaSeparatedText, .spreadsheet, .data], allowsMultipleSelection: true) { r in
            guard case .success(let urls) = r else { return }
            for url in urls {
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                if let d = try? Data(contentsOf: url) { model.attach(name: url.lastPathComponent, data: d) }
            }
        }
        #if os(iOS)
        .fullScreenCover(isPresented: $camera) {
            CameraPicker { data in
                model.attach(name: "site-\(Int(Date().timeIntervalSince1970)).jpg", data: jpegForUpload(data))
            }
            .ignoresSafeArea()
        }
        #endif
        .quickLookPreview($preview)
    }

    private func open(_ f: FileRef) {
        Task {
            do {
                preview = try await api.download(model.bot.id, file: f)
            } catch {
                model.error = "Could not open \(f.name): \(error.localizedDescription)"
            }
        }
    }

    // MARK: composer

    private var composer: some View {
        VStack(spacing: 6) {
            if model.busy {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(model.activity.isEmpty ? "Working" : model.activity)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .contentTransition(.opacity)
                        .animation(.default, value: model.activity)
                    Spacer()
                    Button("Stop", systemImage: "stop.circle.fill") { model.stop() }
                        .labelStyle(.titleAndIcon)
                        .font(.footnote.weight(.medium))
                        .tint(.red)
                }
                .padding(.horizontal, 14)
            }
            if !model.pending.isEmpty || model.uploading > 0 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(model.pending, id: \.path) { u in
                            HStack(spacing: 4) {
                                Image(systemName: FileRef(kind: "", path: u.path, name: u.name).symbol)
                                Text(u.name).lineLimit(1)
                                Button { model.removePending(u) } label: { Image(systemName: "xmark.circle.fill") }
                                    .accessibilityLabel("Remove \(u.name)")
                            }
                            .font(.caption)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .background(.thinMaterial, in: Capsule())
                        }
                        if model.uploading > 0 {
                            ProgressView().controlSize(.small).padding(.horizontal, 6)
                        }
                    }
                    .padding(.horizontal, 12)
                }
            }
            HStack(alignment: .bottom, spacing: 8) {
                Menu {
                    #if os(iOS)
                    if UIImagePickerController.isSourceTypeAvailable(.camera) {
                        Button("Camera", systemImage: "camera") { camera = true }
                    }
                    #endif
                    Button("Photos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
                    Button("Files", systemImage: "folder") { pickingFiles = true }
                } label: {
                    Image(systemName: "plus.circle.fill")
                        .font(.system(size: 28))
                        .symbolRenderingMode(.hierarchical)
                }
                .accessibilityLabel("Attach")
                TextField("Message \(model.bot.name)", text: $input, axis: .vertical)
                    .lineLimit(1...6)
                    .focused($focused)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.background, in: RoundedRectangle(cornerRadius: 18))
                    .overlay(RoundedRectangle(cornerRadius: 18).strokeBorder(.quaternary))
                Button {
                    model.send(input)
                    input = ""
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 30))
                }
                .disabled(!model.canSend || (input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && model.pending.isEmpty))
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 10)
        }
        .padding(.vertical, 8)
        .background(.bar)
    }
}

// MARK: rows

struct MessageRow: View {
    let item: ChatItem
    let bot: BotInfo
    let api: API
    let open: (FileRef) -> Void
    @State private var expanded = false

    var body: some View {
        switch item.role {
        case .user:
            HStack {
                Spacer(minLength: 48)
                VStack(alignment: .trailing, spacing: 6) {
                    if !item.text.isEmpty {
                        Text(item.text)
                            .textSelection(.enabled)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 9)
                            .foregroundStyle(.white)
                            .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 18))
                    }
                    ForEach(item.files, id: \.path) { f in
                        FileCard(file: f, bot: bot.id, api: api, open: open)
                    }
                }
            }
        case .bot:
            HStack(alignment: .bottom, spacing: 8) {
                Avatar(bot: bot, api: api, size: 28)
                Text(markdown(item.text))
                    .textSelection(.enabled)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18))
                Spacer(minLength: 32)
            }
        case .team:
            VStack(alignment: .leading, spacing: 4) {
                Button {
                    withAnimation(.snappy) { expanded.toggle() }
                } label: {
                    Label("\(item.agent) reported", systemImage: expanded ? "chevron.down" : "chevron.right")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                if expanded {
                    Text(markdown(item.text))
                        .font(.callout)
                        .textSelection(.enabled)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.fill.quaternary, in: RoundedRectangle(cornerRadius: 12))
                }
            }
            .padding(.leading, 36)
            .frame(maxWidth: .infinity, alignment: .leading)
        case .note:
            Text(item.text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .padding(.leading, 36)
                .frame(maxWidth: .infinity, alignment: .leading)
        case .file:
            HStack {
                ForEach(item.files, id: \.path) { f in
                    FileCard(file: f, bot: bot.id, api: api, open: open)
                }
                Spacer()
            }
            .padding(.leading, 36)
        case .error:
            Label(item.text, systemImage: "exclamationmark.triangle.fill")
                .font(.footnote)
                .foregroundStyle(.red)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.leading, 36)
        case .end:
            Text(item.text)
                .font(.caption2.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 3)
                .background(.fill.quaternary, in: Capsule())
        }
    }

    /// Inline Markdown (bold, italics, code, links), keeping the line breaks.
    private func markdown(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}

/// A file the team made or the engineer sent: a picture shows inline, anything else as a card.
struct FileCard: View {
    let file: FileRef
    let bot: String
    let api: API
    let open: (FileRef) -> Void
    @State private var image: PlatformImage?

    var body: some View {
        Button { open(file) } label: {
            if file.isImage, let image {
                Image(platform: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 240, maxHeight: 240)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
            } else {
                HStack(spacing: 10) {
                    Image(systemName: file.symbol)
                        .font(.title3)
                        .frame(width: 28)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(file.name).font(.subheadline.weight(.medium)).lineLimit(1)
                        Text(file.kind).font(.caption).foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Open \(file.name)")
        .task(id: file.path) {
            guard file.isImage, image == nil else { return }
            if let d = try? await api.file(bot, path: file.path) { image = PlatformImage(data: d) }
        }
    }
}

#if os(iOS)
/// The camera, for site photos.
struct CameraPicker: UIViewControllerRepresentable {
    let done: (Data) -> Void
    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let c = UIImagePickerController()
        c.sourceType = .camera
        c.delegate = context.coordinator
        return c
    }

    func updateUIViewController(_ c: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    @MainActor
    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let parent: CameraPicker
        init(_ p: CameraPicker) { parent = p }

        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            if let img = info[.originalImage] as? UIImage, let d = img.jpegData(compressionQuality: 0.9) {
                parent.done(d)
            }
            parent.dismiss()
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            parent.dismiss()
        }
    }
}
#endif
