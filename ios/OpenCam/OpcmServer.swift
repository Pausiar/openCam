import CryptoKit
import Foundation
import Network
import UIKit

/// Servidor OPCM del iPhone en el puerto 4747. Un solo cliente activo: el escritorio.
/// Flujo: HELLO → HELLO → AUTH → AUTH (emparejamiento, en claro) y después, cifrado:
/// CAPS → CONFIG → VIDEO continuo; PING → PONG.
/// Una conexión nueva solo sustituye a la activa cuando se ha autenticado, así que un
/// extraño en la red no puede ver la cámara ni echar al PC conectado.
/// Todo el estado mutable vive en `queue`; los callbacks se invocan en ella.
final class OpcmServer {
    static let port: UInt16 = 4747
    private static let maxPendingHandshakes = 4
    private static let handshakeTimeout: TimeInterval = 10
    /// Frames de vídeo que pueden esperar en el socket (~100 ms a 30 fps) antes de descartar.
    private static let maxPendingVideo = 3

    /// Una conexión TCP y su estado de handshake/cifrado.
    private final class Peer {
        enum Phase { case hello, auth, ready }
        let conn: NWConnection
        var buffer = Data()
        var phase = Phase.hello
        var seq: UInt32 = 0
        var closing = false
        var desktopHello = Data()
        var phoneHello = Data()
        var clientId = ""
        var usingSavedKey = false
        var keys: Secure.SessionKeys?
        var tx: Secure.GcmCipher?
        var rx: Secure.GcmCipher?
        /// Frames de vídeo entregados a la conexión y aún sin salir por la red.
        var pendingVideo = 0
        /// Tras un descarte no se envían P-frames hasta el siguiente keyframe.
        var waitingKey = false
        init(_ conn: NWConnection) { self.conn = conn }
    }

    private let queue = DispatchQueue(label: "opencam.server")
    private var listener: NWListener?
    private var active: Peer?
    private var pending: [ObjectIdentifier: Peer] = [:]
    private let epoch = DispatchTime.now().uptimeNanoseconds
    private let deviceName = String(UIDevice.current.name.prefix(63))
    let pairing = Pairing()

    /// Se llama cuando el cliente envía CONFIG; el controlador arranca la captura.
    var onConfig: ((Data) -> Void)?
    /// Se llama cuando el escritorio pide un keyframe.
    var onKeyframeRequest: (() -> Void)?
    /// STATS (0x22): JSON con `target_bitrate_kbps`.
    var onStats: ((Data) -> Void)?
    var onClientConnected: (() -> Void)?
    var onClientDisconnected: (() -> Void)?
    /// CAPS que se anuncian tras el HELLO; los fija el streamer según el hardware.
    var capsPayload = Data(#"{"platform":"ios","codecs":["h264"],"lenses":["wide"]}"#.utf8)

    /// Código de emparejamiento visible (6 dígitos). Cambia tras emparejar o tras varios fallos.
    var pairingCode: String { queue.sync { pairing.code } }
    var onPairingCodeChanged: ((String) -> Void)? {
        get { queue.sync { pairing.onCodeChanged } }
        set { queue.sync { pairing.onCodeChanged = newValue } }
    }

    func start() throws {
        let listener = try NWListener(using: .tcp, on: NWEndpoint.Port(rawValue: Self.port)!)
        // Bonjour: el escritorio descubre el iPhone por `_opencam._tcp`.
        listener.service = NWListener.Service(name: deviceName, type: "_opencam._tcp",
                                              txtRecord: Self.txt(["platform": "ios", "name": deviceName]))
        listener.newConnectionHandler = { [weak self] conn in self?.accept(conn) }
        listener.start(queue: queue)
        queue.sync { self.listener = listener }
    }

    func stop() {
        queue.sync {
            listener?.cancel()
            listener = nil
            let peers = Array(pending.values) + (active.map { [$0] } ?? [])
            pending.removeAll()
            active = nil
            peers.forEach { $0.conn.cancel() }
        }
    }

    func sendError(code: String, message: String) {
        queue.async { [weak self] in
            guard let self, let p = self.active else { return }
            self.send(p, .error, payload: Self.json(["code": code, "message": message]))
        }
    }

    /// Envía un access unit H.264 (Annex-B) como VIDEO. Llamar desde cualquier hilo.
    /// Si la red va más lenta que la cámara, los frames no se acumulan (crecerían la latencia y
    /// la memoria sin límite): se descartan hasta el siguiente keyframe y se pide uno.
    func sendVideo(_ accessUnit: Data, keyframe: Bool, timestampUs: UInt64) {
        queue.async { [weak self] in
            guard let self, let p = self.active else { return }
            if p.pendingVideo >= Self.maxPendingVideo {
                p.waitingKey = true
                return
            }
            if keyframe {
                p.waitingKey = false
            } else if p.waitingKey {
                return
            }
            p.pendingVideo += 1
            let data = self.frame(p, .video, flags: keyframe ? OPCM.keyframeFlag : 0,
                                  timestampUs: timestampUs, payload: accessUnit)
            // El aviso de envío llega en `queue` (la cola de la conexión).
            p.conn.send(content: data, completion: .contentProcessed { [weak self, weak p] _ in
                guard let self, let p else { return }
                p.pendingVideo -= 1
                // Cola vacía tras un descarte: se reanuda cuanto antes con un keyframe.
                if p.waitingKey, p.pendingVideo == 0, self.active === p { self.onKeyframeRequest?() }
            })
        }
    }

    /// Registro TXT DNS-SD: cada entrada es longitud (1 byte) + "clave=valor".
    private static func txt(_ kv: [String: String]) -> Data {
        var d = Data()
        for (k, v) in kv {
            let e = Data("\(k)=\(v)".utf8.prefix(255))
            d.append(UInt8(e.count))
            d.append(e)
        }
        return d
    }

    private static func json(_ obj: [String: Any]) -> Data {
        (try? JSONSerialization.data(withJSONObject: obj)) ?? Data("{}".utf8)
    }

    // MARK: - Conexiones (todo en `queue`)

    private func accept(_ conn: NWConnection) {
        guard pending.count < Self.maxPendingHandshakes else { conn.cancel(); return }
        let peer = Peer(conn)
        pending[ObjectIdentifier(peer)] = peer
        conn.stateUpdateHandler = { [weak self, weak peer] state in
            guard let self, let peer else { return }
            switch state {
            case .failed, .cancelled: self.drop(peer)
            default: break
            }
        }
        conn.start(queue: queue)
        // Un handshake que no termina a tiempo se corta: no ocupa hueco para siempre.
        queue.asyncAfter(deadline: .now() + Self.handshakeTimeout) { [weak peer] in
            if let peer, peer.phase != .ready { peer.conn.cancel() }
        }
        receive(peer)
    }

    /// Ignora conexiones viejas: su .cancelled puede llegar cuando ya hay otro cliente.
    private func drop(_ peer: Peer) {
        pending.removeValue(forKey: ObjectIdentifier(peer))
        guard active === peer else { return }
        active = nil
        onClientDisconnected?()
    }

    private func receive(_ peer: Peer) {
        peer.conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty, !peer.closing {
                peer.buffer.append(data)
                self.drainMessages(peer)
            }
            if isComplete || error != nil {
                peer.conn.cancel()
                return
            }
            self.receive(peer)
        }
    }

    /// Extrae mensajes completos del buffer. Valida magic, versión y tamaño antes de usar el payload.
    private func drainMessages(_ peer: Peer) {
        while peer.buffer.count >= OPCM.headerLen, !peer.closing {
            if Array(peer.buffer.prefix(4)) == OPCM.magic, peer.buffer[peer.buffer.startIndex + 4] != OPCM.version {
                close(peer, code: "unsupported_version", message: "solo se soporta la versión 1")
                return
            }
            guard let h = OPCM.Header.decode(peer.buffer) else {
                // Cabecera inválida: el stream está desincronizado, se corta la conexión.
                peer.conn.cancel()
                return
            }
            // Antes de autenticarse nadie puede hacer reservar mucha memoria.
            let limit = peer.phase == .ready ? OPCM.maxPayload : Secure.maxUnauthPayload
            guard h.payloadLen <= limit else { peer.conn.cancel(); return }
            let total = OPCM.headerLen + Int(h.payloadLen)
            guard peer.buffer.count >= total else { return }
            let header = Data(peer.buffer.prefix(OPCM.headerLen))
            var payload = Data(peer.buffer[(peer.buffer.startIndex + OPCM.headerLen)..<(peer.buffer.startIndex + total)])
            peer.buffer.removeSubrange(peer.buffer.startIndex..<(peer.buffer.startIndex + total))
            switch peer.phase {
            case .hello: handleHello(peer, type: h.msgType, payload: payload)
            case .auth: handleAuth(peer, type: h.msgType, payload: payload)
            case .ready:
                // Un mensaje manipulado, repetido o sin cifrar corta la conexión.
                guard let rx = peer.rx, let plain = try? rx.open(header: header, ciphertext: payload) else {
                    peer.conn.cancel()
                    return
                }
                payload = plain
                handle(peer, type: h.msgType, timestampUs: h.timestampUs, payload: payload)
            }
        }
    }

    private func handleHello(_ peer: Peer, type: UInt8, payload: Data) {
        guard type == OPCM.MsgType.hello.rawValue else { peer.conn.cancel(); return }
        let req = (try? JSONSerialization.jsonObject(with: payload)) as? [String: Any] ?? [:]
        guard (req["secure"] as? Int) == 1 else {
            close(peer, code: "pairing_required",
                  message: "Este iPhone exige emparejamiento cifrado: actualiza OpenCam en el PC")
            return
        }
        guard let clientId = req["client_id"] as? String, Secure.isId(clientId),
              let peerPub = (req["pub"] as? String).flatMap(Secure.unhex) else {
            close(peer, code: "bad_hello", message: "HELLO incompleto")
            return
        }
        let saved = (req["auth"] as? String) == "key" ? pairing.key(for: clientId) : nil
        if saved == nil {
            // Tras varios códigos fallidos seguidos se deja de aceptar código durante un rato.
            let wait = pairing.lockedForSeconds
            if wait > 0 {
                close(peer, code: "too_many_attempts",
                      message: "Demasiados códigos incorrectos: espera \(wait) s y usa el código nuevo que muestra el iPhone")
                return
            }
        }
        let psk = saved ?? Secure.codePsk(pairing.code)!
        let kp = P256Key()
        let reply = Self.json(["proto": 1, "platform": "ios", "name": deviceName, "secure": 1,
                               "phone_id": pairing.phoneId,
                               "pub": Secure.hex(kp.publicBytes),
                               "auth": saved != nil ? "key" : "code"])
        guard let keys = try? Secure.SessionKeys.derive(own: kp.key, peerPublic: peerPub, psk: psk,
                                                        desktopHello: payload, phoneHello: reply) else {
            close(peer, code: "bad_hello", message: "clave pública inválida")
            return
        }
        peer.clientId = clientId
        peer.usingSavedKey = saved != nil
        peer.keys = keys
        peer.phase = .auth
        sendPlain(peer, .hello, payload: reply)
    }

    private func handleAuth(_ peer: Peer, type: UInt8, payload: Data) {
        guard type == OPCM.MsgType.auth.rawValue, let keys = peer.keys else { peer.conn.cancel(); return }
        guard keys.verifyAuth(.desktop, tag: payload) else {
            if peer.usingSavedKey {
                close(peer, code: "bad_key", message: "este PC ya no está emparejado; usa el código")
            } else {
                pairing.onFailure()
                close(peer, code: "bad_code", message: "código de emparejamiento incorrecto")
            }
            return
        }
        // Se guarda la clave antes de confirmar: cuando el PC recibe AUTH ya puede reconectar con ella.
        if !peer.usingSavedKey { pairing.onPaired(clientId: peer.clientId, key: keys.pairingKey) }
        sendPlain(peer, .auth, payload: keys.authTag(.phone))
        (peer.tx, peer.rx) = keys.ciphers(.phone)
        peer.phase = .ready
        promote(peer)
    }

    /// El cliente autenticado pasa a ser el activo y sustituye al anterior.
    private func promote(_ peer: Peer) {
        pending.removeValue(forKey: ObjectIdentifier(peer))
        if let old = active {
            active = nil
            old.conn.cancel()
            onClientDisconnected?()
        }
        active = peer
        onClientConnected?()
        send(peer, .caps, payload: capsPayload)
    }

    /// ERROR en claro (o cifrado si ya hay sesión) y cierre una vez enviado.
    private func close(_ peer: Peer, code: String, message: String) {
        peer.closing = true
        peer.buffer.removeAll()
        let data = frame(peer, .error, flags: 0, timestampUs: currentTimestampUs(),
                         payload: Self.json(["code": code, "message": message]))
        peer.conn.send(content: data, completion: .contentProcessed { _ in peer.conn.cancel() })
    }

    private func handle(_ peer: Peer, type: UInt8, timestampUs: UInt64, payload: Data) {
        guard let t = OPCM.MsgType(rawValue: type) else { return }
        switch t {
        case .hello:
            send(peer, .caps, payload: capsPayload)
        case .config:
            onConfig?(payload)
        case .ping:
            // El PONG repite el timestamp del PING para que el escritorio calcule la latencia.
            send(peer, .pong, timestampUs: timestampUs, payload: Data())
        case .keyframeRequest:
            onKeyframeRequest?()
        case .stats:
            onStats?(payload)
        default:
            break
        }
    }

    private func sendPlain(_ peer: Peer, _ t: OPCM.MsgType, payload: Data) {
        peer.conn.send(content: frame(peer, t, flags: 0, timestampUs: currentTimestampUs(), payload: payload),
                       completion: .contentProcessed { _ in })
    }

    private func send(_ peer: Peer, _ t: OPCM.MsgType, flags: UInt16 = 0, timestampUs: UInt64? = nil, payload: Data) {
        peer.conn.send(content: frame(peer, t, flags: flags, timestampUs: timestampUs ?? currentTimestampUs(), payload: payload),
                       completion: .contentProcessed { _ in })
    }

    /// Serializa un mensaje; tras AUTH el payload va cifrado y la cabecera autenticada.
    private func frame(_ peer: Peer, _ t: OPCM.MsgType, flags: UInt16, timestampUs: UInt64, payload: Data) -> Data {
        defer { peer.seq &+= 1 }
        guard let tx = peer.tx else {
            return OPCM.frame(t, seq: peer.seq, timestampUs: timestampUs, flags: flags, payload: payload)
        }
        let header = OPCM.Header(msgType: t.rawValue, flags: flags, seq: peer.seq, timestampUs: timestampUs,
                                 payloadLen: UInt32(payload.count + Secure.tagLen)).encode()
        guard let ct = try? tx.seal(header: header, plaintext: payload) else { peer.conn.cancel(); return Data() }
        return header + ct
    }

    private func currentTimestampUs() -> UInt64 {
        (DispatchTime.now().uptimeNanoseconds - epoch) / 1000
    }
}

/// Par de claves ECDH efímero.
private struct P256Key {
    let key = P256.KeyAgreement.PrivateKey()
    /// 0x04 || X || Y.
    var publicBytes: Data { key.publicKey.x963Representation }
}
