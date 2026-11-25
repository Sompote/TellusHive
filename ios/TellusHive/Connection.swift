import Foundation
import Observation
import Security

/// The server address (in the app's settings) and token (in the Keychain).
@Observable
@MainActor
final class Connection {
    private(set) var api: API?
    var address: String
    var token: String

    private static let addressKey = "serverAddress"
    private static let service = "com.tellushive.mobile.token"

    init() {
        address = UserDefaults.standard.string(forKey: Self.addressKey) ?? ""
        token = Self.readToken() ?? ""
        api = Self.make(address, token)
    }

    var isConfigured: Bool { api != nil }

    /// Checks the server answers with this token, then keeps both.
    func connect(address: String, token: String) async throws {
        let address = Self.normalise(address)
        let token = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let api = Self.make(address, token) else { throw APIError.badURL }
        _ = try await api.info()
        UserDefaults.standard.set(address, forKey: Self.addressKey)
        Self.saveToken(token)
        self.address = address
        self.token = token
        self.api = api
    }

    func disconnect() {
        UserDefaults.standard.removeObject(forKey: Self.addressKey)
        Self.saveToken(nil)
        address = ""
        token = ""
        api = nil
    }

    /// "myserver.ts.net" → "https://myserver.ts.net"
    static func normalise(_ s: String) -> String {
        var s = s.trimmingCharacters(in: .whitespacesAndNewlines)
        while s.hasSuffix("/") { s.removeLast() }
        if !s.isEmpty && !s.contains("://") { s = "https://" + s }
        return s
    }

    private static func make(_ address: String, _ token: String) -> API? {
        guard !token.isEmpty, let url = URL(string: address), url.scheme != nil, url.host != nil else { return nil }
        return API(base: url, token: token)
    }

    private static func readToken() -> String? {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
        var out: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data else { return nil }
        return String(data: d, encoding: .utf8)
    }

    private static func saveToken(_ token: String?) {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
        SecItemDelete(q as CFDictionary)
        guard let token else { return }
        var add = q
        add[kSecValueData as String] = Data(token.utf8)
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        SecItemAdd(add as CFDictionary, nil)
    }
}
