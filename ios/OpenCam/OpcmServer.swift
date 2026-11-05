import Foundation
import Network
import UIKit

/// Servidor OPCM del iPhone en el puerto 4747. Un solo cliente a la vez: el escritorio.
/// Flujo: HELLO → HELLO+CAPS → CONFIG → VIDEO continuo; PING → PONG.
/// Todo el estado mutable vive en `queue`; los callbacks se invocan en ella.
final class OpcmServer {
    static let port: UInt16 = 4747

    private let queue = DispatchQueue(label: "opencam.server")
    private var listener: NWListener?
    private var connection: NWConnection?
    private var seq: UInt32 = 0
    private var buffer = Data()
    private var closing = false
    private let epoch = DispatchTime.now().uptimeNanoseconds
    private let deviceName = String(UIDevice.current.name.prefix(63))

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
            let c = connection
            connection = nil
            seq = 0
            buffer.removeAll()
            closing = false
            c?.cancel()
        }
    }

    func sendError(code: String, message: String) {
        queue.async { [weak self] in
            self?.send(.error, payload: Self.json(["code": code, "message": message]))
        }
    }

    /// Envía un access unit H.264 (Annex-B) como VIDEO. Llamar desde cualquier hilo.
    func sendVideo(_ accessUnit: Data, keyframe: Bool, timestampUs: UInt64) {
        queue.async { [weak self] in
            guard let self, let conn = self.connection else { return }
            conn.send(content: self.nextFrame(.video, flags: keyframe ? OPCM.keyframeFlag : 0,
                                              timestampUs: timestampUs, payload: accessUnit),
                      completion: .contentProcessed { _ in })
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

    private func accept(_ conn: NWConnection) {
        // Solo un cliente: si ya hay uno, se rechaza el nuevo.
        if connection != nil { conn.cancel(); return }
        connection = conn
        closing = false
        conn.stateUpdateHandler = { [weak self] state in
            if case .failed = state { self?.dropClient(conn) }
            if case .cancelled = state { self?.dropClient(conn) }
        }
        conn.start(queue: queue)
        receive(on: conn)
        onClientConnected?()
    }

    /// Ignora conexiones viejas: su .cancelled puede llegar cuando ya hay otro cliente.
    private func dropClient(_ conn: NWConnection) {
        guard connection === conn else { return }
        connection = nil
        buffer.removeAll()
        onClientDisconnected?()
    }

    private func receive(on conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty, !self.closing {
                self.buffer.append(data)
                self.drainMessages()
            }
            if isComplete || error != nil {
                conn.cancel()
                return
            }
            self.receive(on: conn)
        }
    }

    /// Extrae mensajes completos del buffer. Valida magic, versión y tamaño antes de usar el payload.
    private func drainMessages() {
        while buffer.count >= OPCM.headerLen, !closing {
            if Array(buffer.prefix(4)) == OPCM.magic, buffer[buffer.startIndex + 4] != OPCM.version {
                rejectVersion()
                return
            }
            guard let h = OPCM.Header.decode(buffer) else {
                // Cabecera inválida: el stream está desincronizado, se corta la conexión.
                connection?.cancel()
                return
            }
            guard h.payloadLen <= OPCM.maxPayload else { connection?.cancel(); return }
            let total = OPCM.headerLen + Int(h.payloadLen)
            guard buffer.count >= total else { return }
            let payload = buffer.subdata(in: OPCM.headerLen..<total)
            buffer.removeSubrange(0..<total)
            handle(type: h.msgType, timestampUs: h.timestampUs, payload: payload)
        }
    }

    /// Versión no soportada: ERROR `unsupported_version` y cierre (docs/PROTOCOL.md).
    private func rejectVersion() {
        guard let conn = connection else { return }
        closing = true
        buffer.removeAll()
        let msg = Self.json(["code": "unsupported_version", "message": "solo se soporta la versión 1"])
        conn.send(content: nextFrame(.error, flags: 0, timestampUs: currentTimestampUs(), payload: msg),
                  completion: .contentProcessed { _ in conn.cancel() })
    }

    private func handle(type: UInt8, timestampUs: UInt64, payload: Data) {
        guard let t = OPCM.MsgType(rawValue: type) else { return }
        switch t {
        case .hello:
            send(.hello, payload: Self.json(["proto": 1, "platform": "ios", "name": deviceName]))
            send(.caps, payload: capsPayload)
        case .config:
            onConfig?(payload)
        case .ping:
            // El PONG repite el timestamp del PING para que el escritorio calcule la latencia.
            sendRaw(.pong, timestampUs: timestampUs, payload: Data())
        case .keyframeRequest:
            onKeyframeRequest?()
        case .stats:
            onStats?(payload)
        default:
            break
        }
    }

    private func send(_ t: OPCM.MsgType, payload: Data) {
        sendRaw(t, timestampUs: currentTimestampUs(), payload: payload)
    }

    private func sendRaw(_ t: OPCM.MsgType, timestampUs: UInt64, payload: Data) {
        guard let conn = connection else { return }
        conn.send(content: nextFrame(t, flags: 0, timestampUs: timestampUs, payload: payload),
                  completion: .contentProcessed { _ in })
    }

    private func nextFrame(_ t: OPCM.MsgType, flags: UInt16, timestampUs: UInt64, payload: Data) -> Data {
        let data = OPCM.frame(t, seq: seq, timestampUs: timestampUs, flags: flags, payload: payload)
        seq &+= 1
        return data
    }

    private func currentTimestampUs() -> UInt64 {
        (DispatchTime.now().uptimeNanoseconds - epoch) / 1000
    }
}
