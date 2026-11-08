/*
 * SecureStore.swift — PLAT-002 secure storage for the Carry-On macOS app:
 * the device identity's PRIVATE KEY DER, its certificate DER, and the
 * TrustStore JSON live in the macOS **Keychain** (Security framework, §8.7).
 * Never a file, never UserDefaults, never a log line (EVD-006/§22.3).
 *
 * Items are generic-password Keychain entries (service "com.carryon.identity"):
 * the OS keys them to the app's code-signing identity and user session, and
 * SecItemCopyMatching fails closed on tamper (errSecItemNotFound /
 * errSecAuthFailed) — the store then regenerates a FRESH identity rather than
 * trusting damaged credentials (§18.4: a bad restore fails, never half-trusts).
 *
 * Loaded LAZILY at first use so app start never blocks on Keychain (stage-5
 * optimization requirement). Everything here runs off the main thread.
 */

import Foundation
import Security

public enum SecureStore {

    private static let service = "com.carryon.identity"
    private static let accountKey = "device-key"
    private static let accountCert = "device-cert"
    private static let accountTrust = "device-trust"

    // ---- Keychain primitives (generic password items holding raw bytes) ----

    private static func write(_ data: Data, account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var add = query
            attributes.forEach { add[$0.key] = $0.value }
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            throw CarryonError(code: -1, message: "Keychain write (\(account)): OSStatus \(status)")
        }
    }

    private static func read(account: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess else { return nil } // fail closed
        return out as? Data
    }

    private static func delete(account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }

    // ---- the durable device identity ---------------------------------------

    /// The durable identity: restore from Keychain-held DERs (stable pin
    /// across launches, §18.4) or generate + persist a fresh one. The key DER
    /// exists in memory only inside the engine after this returns.
    public static func loadOrCreateIdentity(name: String) throws -> DeviceIdentity {
        if let cert = read(account: accountCert), let key = read(account: accountKey) {
            if let id = try? DeviceIdentity.fromDER(name: name, cert: cert, key: key) {
                return id
            }
            // Unparseable/tampered restore fails closed: drop + regenerate.
            delete(account: accountCert)
            delete(account: accountKey)
        }
        let id = try DeviceIdentity.generate(name: name)
        // The ONLY two lines that ever touch the SECRET; straight to Keychain.
        try write(try id.keyDER(), account: accountKey)
        try write(try id.certDER(), account: accountCert)
        return id
    }

    // ---- the trust store (pins only, but Keychain too, per the brief) -------

    public static func saveTrust(json: String) throws {
        try write(Data(json.utf8), account: accountTrust)
    }

    public static func loadTrust() -> TrustStore? {
        guard let data = read(account: accountTrust),
              let json = String(data: data, encoding: .utf8),
              let trust = try? TrustStore.fromJSON(json) else { return nil }
        return trust
    }
}
