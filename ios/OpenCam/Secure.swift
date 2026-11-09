import CryptoKit
import Foundation
import Security

/// Emparejamiento y cifrado OPCM (docs/PROTOCOL.md, «Seguridad»), igual que
/// `core/src/secure.rs` y `Secure.kt`: ECDH P-256 → HKDF-SHA256 (sal = PSK) → AUTH con
/// HMAC → AES-256-GCM con la cabecera OPCM como dato autenticado.
enum Secure {
    static let tagLen = 16
    static let maxUnauthPayload = 64 * 1024
    static let maxCodeAttempts = 5

    private static let info = Data("opencam-v1".utf8)
    private static let codeSalt = Data("opencam-pair-code:".utf8)

    enum Role: String {
        case desktop, phone
    }

    struct Failure: Error {}

    /// «123 456» o «123-456» → «123456»; nil si no son 6 dígitos.
    static func normalizeCode(_ code: String) -> String? {
        let d = code.filter { !$0.isWhitespace && $0 != "-" }
        guard d.count == 6, d.allSatisfy({ ("0"..."9").contains($0) }) else { return nil }
        return d
    }

    static func codePsk(_ code: String) -> Data? {
        guard let c = normalizeCode(code) else { return nil }
        return Data(SHA256.hash(data: codeSalt + Data(c.utf8)))
    }

    /// Código uniforme de 6 dígitos (SystemRandomNumberGenerator es criptográfico).
    static func newCode() -> String {
        String(format: "%06d", Int.random(in: 0..<1_000_000))
    }

    static func randomBytes(_ n: Int) -> Data {
        Data((0..<n).map { _ in UInt8.random(in: 0...255) })
    }

    static func transcriptHash(_ desktopHello: Data, _ phoneHello: Data) -> Data {
        var d = Data()
        d.append(contentsOf: withUnsafeBytes(of: UInt32(desktopHello.count).bigEndian) { Array($0) })
        d.append(desktopHello)
        d.append(contentsOf: withUnsafeBytes(of: UInt32(phoneHello.count).bigEndian) { Array($0) })
        d.append(phoneHello)
        return Data(SHA256.hash(data: d))
    }

    struct SessionKeys {
        let desktopToPhone: SymmetricKey
        let phoneToDesktop: SymmetricKey
        let confirm: SymmetricKey
        let pairingKey: Data

        static func derive(own: P256.KeyAgreement.PrivateKey, peerPublic: Data, psk: Data,
                           desktopHello: Data, phoneHello: Data) throws -> SessionKeys {
            guard peerPublic.count == 65, peerPublic.first == 0x04 else { throw Failure() }
            // CryptoKit rechaza puntos fuera de la curva.
            let peer = try P256.KeyAgreement.PublicKey(x963Representation: peerPublic)
            let shared = try own.sharedSecretFromKeyAgreement(with: peer)
            let okm = shared.hkdfDerivedSymmetricKey(
                using: SHA256.self, salt: psk,
                sharedInfo: info + transcriptHash(desktopHello, phoneHello),
                outputByteCount: 128
            ).withUnsafeBytes { Data($0) }
            func part(_ i: Int) -> Data { Data(okm[(i * 32)..<((i + 1) * 32)]) }
            return SessionKeys(
                desktopToPhone: SymmetricKey(data: part(0)),
                phoneToDesktop: SymmetricKey(data: part(1)),
                confirm: SymmetricKey(data: part(2)),
                pairingKey: part(3)
            )
        }

        func authTag(_ role: Role) -> Data {
            Data(HMAC<SHA256>.authenticationCode(for: Data(role.rawValue.utf8), using: confirm))
        }

        func verifyAuth(_ role: Role, tag: Data) -> Bool {
            HMAC<SHA256>.isValidAuthenticationCode(tag, authenticating: Data(role.rawValue.utf8), using: confirm)
        }

        /// (envío, recepción) para `role`.
        func ciphers(_ role: Role) -> (GcmCipher, GcmCipher) {
            let d2p = GcmCipher(key: desktopToPhone)
            let p2d = GcmCipher(key: phoneToDesktop)
            return role == .desktop ? (d2p, p2d) : (p2d, d2p)
        }
    }

    /// AES-256-GCM en un sentido; nonce = 0⁴ || contador u64 big-endian implícito.
    final class GcmCipher {
        private let key: SymmetricKey
        private var counter: UInt64 = 0

        init(key: SymmetricKey) { self.key = key }

        private func nextNonce() throws -> AES.GCM.Nonce {
            guard counter < UInt64.max else { throw Failure() }
            var n = Data(count: 4)
            n.append(contentsOf: withUnsafeBytes(of: counter.bigEndian) { Array($0) })
            counter += 1
            return try AES.GCM.Nonce(data: n)
        }

        /// `header` ya lleva payload_len = plaintext + 16.
        func seal(header: Data, plaintext: Data) throws -> Data {
            let box = try AES.GCM.seal(plaintext, using: key, nonce: nextNonce(), authenticating: header)
            return box.ciphertext + box.tag
        }

        func open(header: Data, ciphertext: Data) throws -> Data {
            guard ciphertext.count >= tagLen else { throw Failure() }
            let ct = Data(ciphertext)
            let box = try AES.GCM.SealedBox(nonce: nextNonce(),
                                            ciphertext: ct.prefix(ct.count - tagLen),
                                            tag: ct.suffix(tagLen))
            return try AES.GCM.open(box, using: key, authenticating: header)
        }
    }

    static func hex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined() }

    static func unhex(_ s: String) -> Data? {
        let chars = Array(s.utf8)
        guard chars.count % 2 == 0 else { return nil }
        var out = Data(capacity: chars.count / 2)
        var i = 0
        while i < chars.count {
            guard let b = UInt8(String(decoding: chars[i..<(i + 2)], as: UTF8.self), radix: 16) else { return nil }
            out.append(b)
            i += 2
        }
        return out
    }

    /// Ids hex de 4 a 64 caracteres.
    static func isId(_ s: String) -> Bool {
        (4...64).contains(s.count) && s.allSatisfy { $0.isHexDigit && $0.isASCII }
    }
}

/// Estado de emparejamiento del iPhone: id propio, código visible y claves de los PCs
/// (en el llavero, solo en este dispositivo). El código cambia tras emparejar y tras
/// `Secure.maxCodeAttempts` fallos. Cada tanda de fallos bloquea además el emparejamiento por
/// código un rato que se duplica (30 s, 1 min… hasta 10 min), para que nadie en la red pueda
/// probar códigos sin parar. Se usa solo desde la cola del servidor.
final class Pairing {
    private(set) var phoneId: String
    private(set) var code = Secure.newCode()
    private var failures = 0
    private var lockouts = 0
    private var lockedUntil: UInt64 = 0

    private static let lockoutBase: UInt64 = 30
    private static let lockoutMax: UInt64 = 600

    /// Segundos que faltan para volver a aceptar un código (0 = se acepta ya).
    var lockedForSeconds: UInt64 {
        let now = DispatchTime.now().uptimeNanoseconds / 1_000_000_000
        return lockedUntil > now ? lockedUntil - now : 0
    }
    private var keys: [String: String]
    var onCodeChanged: ((String) -> Void)?

    private static let service = "dev.opencam.pairing"

    init() {
        let stored = Pairing.load()
        keys = stored["keys"] as? [String: String] ?? [:]
        phoneId = (stored["phone_id"] as? String).flatMap { Secure.isId($0) ? $0 : nil }
            ?? Secure.hex(Secure.randomBytes(16))
        save()
    }

    func key(for clientId: String) -> Data? {
        keys[clientId].flatMap(Secure.unhex).flatMap { $0.count == 32 ? $0 : nil }
    }

    func onFailure() {
        failures += 1
        guard failures >= Secure.maxCodeAttempts else { return }
        lockouts += 1
        let wait = min(Pairing.lockoutBase << UInt64(min(lockouts - 1, 10)), Pairing.lockoutMax)
        lockedUntil = DispatchTime.now().uptimeNanoseconds / 1_000_000_000 + wait
        rotate()
    }

    func onPaired(clientId: String, key: Data) {
        keys[clientId] = Secure.hex(key)
        save()
        lockouts = 0
        lockedUntil = 0
        rotate()
    }

    func forgetAll() {
        keys.removeAll()
        save()
        rotate()
    }

    private func rotate() {
        failures = 0
        code = Secure.newCode()
        onCodeChanged?(code)
    }

    private static func load() -> [String: Any] {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                kSecAttrService as String: service,
                                kSecReturnData as String: true]
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let d = out as? Data,
              let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else { return [:] }
        return obj
    }

    private func save() {
        guard let d = try? JSONSerialization.data(withJSONObject: ["phone_id": phoneId, "keys": keys]) else { return }
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                kSecAttrService as String: Pairing.service]
        SecItemDelete(q as CFDictionary)
        var add = q
        add[kSecValueData as String] = d
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }
}
