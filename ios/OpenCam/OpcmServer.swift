import Foundation
import Network

/// Servidor OPCM del iPhone en el puerto 4747. Un solo cliente a la vez: el escritorio.
/// Flujo: HELLO → HELLO+CAPS → CONFIG → VIDEO continuo; PING → PONG.
final class OpcmServer {
    static let port: UInt16 = 4747

    private let queue = DispatchQueue(label: "opencam.server")
    private var listener: NWListener?
    private var connection: NWConnection?
    private var seq: UInt32 = 0
    private var buffer = Data()
    private let epoch = DispatchTime.now().uptimeNanoseconds

    /// Se llama cuando el cliente envía CONFIG; el controlador arranca la captura.
    var onConfig: ((Data) -> Void)?
    /// Se llama cuando el escritorio pide un keyframe.
    var onKeyframeRequest: (() -> Void)?

    var isClientConnected: Bool { connection != nil }

    func start() throws {
        let listener = try NWListener(using: .tcp, on: NWEndpoint.Port(rawValue: Self.port)!)
        listener.newConnectionHandler = { [weak self] conn in self?.accept(conn) }
        listener.start(queue: queue)
        self.listener = listener
    }

    func stop() {
        listener?.cancel()
        connection?.cancel()
        listener = nil
        connection = nil
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

    private func accept(_ conn: NWConnection) {
        // Solo un cliente: si ya hay uno, se rechaza el nuevo.
        if connection != nil { conn.cancel(); return }
        connection = conn
        conn.stateUpdateHandler = { [weak self] state in
            if case .failed = state { self?.dropClient() }
            if case .cancelled = state { self?.dropClient() }
        }
        conn.start(queue: queue)
        receive(on: conn)
    }

    private func dropClient() {
        connection = nil
        buffer.removeAll()
    }

    private func receive(on conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty {
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
        while buffer.count >= OPCM.headerLen {
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

    private func handle(type: UInt8, timestampUs: UInt64, payload: Data) {
        guard let t = OPCM.MsgType(rawValue: type) else { return }
        switch t {
        case .hello:
            send(.hello, payload: Data(#"{"proto":1,"platform":"ios","name":"OpenCam"}"#.utf8))
            send(.caps, payload: Data(#"{"platform":"ios","codecs":["h264"],"lenses":["wide"]}"#.utf8))
        case .config:
            onConfig?(payload)
        case .ping:
            // El PONG repite el timestamp del PING para que el escritorio calcule la latencia.
            sendRaw(.pong, timestampUs: timestampUs, payload: Data())
        case .keyframeRequest:
            onKeyframeRequest?()
        default:
            break
        }
    }

    private func send(_ t: OPCM.MsgType, payload: Data) {
        guard let conn = connection else { return }
        conn.send(content: nextFrame(t, flags: 0, timestampUs: currentTimestampUs(), payload: payload),
                  completion: .contentProcessed { _ in })
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
