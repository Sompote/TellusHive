import SwiftUI

@main
@MainActor
struct TellusHiveApp: App {
    @State private var connection = Connection()

    var body: some Scene {
        WindowGroup {
            Group {
                if let api = connection.api {
                    BotListView(api: api)
                } else {
                    NavigationStack { ConnectView() }
                }
            }
            .environment(connection)
        }
    }
}

/// The server address and token. Shown first, and later from the bot list's settings.
@MainActor
struct ConnectView: View {
    @Environment(Connection.self) private var connection
    @Environment(\.dismiss) private var dismiss
    @State private var address = ""
    @State private var token = ""
    @State private var checking = false
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                TextField("https://tellushive.example.ts.net", text: $address)
                    .urlField()
                SecureField("Token", text: $token)
                    .plainField()
            } header: {
                Text("Server")
            } footer: {
                Text("The address of `tellushive serve` on your cloud machine, behind HTTPS (Tailscale or Cloudflare Tunnel). The server prints the token when it starts; it is kept in this iPhone's Keychain.")
            }
            if let error {
                Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
            }
            Section {
                Button {
                    Task { await connect() }
                } label: {
                    HStack {
                        Text("Connect")
                        Spacer()
                        if checking { ProgressView() }
                    }
                }
                .disabled(checking || address.isEmpty || token.isEmpty)
            }
            if connection.isConfigured {
                Section {
                    Button("Disconnect", role: .destructive) {
                        connection.disconnect()
                    }
                }
            }
        }
        .navigationTitle("TellusHive")
        .onAppear {
            address = connection.address
            token = connection.token
        }
    }

    private func connect() async {
        checking = true
        defer { checking = false }
        do {
            try await connection.connect(address: address, token: token)
            error = nil
            Haptics.success()
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
