import Foundation

/// Framing OPCM v1 (docs/PROTOCOL.md): cabecera fija de 24 bytes big-endian + payload.
enum OPCM {
    static let magic: [UInt8] = [0x4F, 0x50, 0x43, 0x4D] // "OPCM"
    static let version: UInt8 = 1
    static let headerLen = 24
    static let maxPayload = 16 * 1024 * 1024
    static let keyframeFlag: UInt16 = 0x0001

    enum MsgType: UInt8 {
        case hello = 0x01
        case caps = 0x02
        case config = 0x03
        case video = 0x10
        case ping = 0x20
        case pong = 0x21
        case stats = 0x22
        case keyframeRequest = 0x30
        case error = 0x7F
    }

    struct Header {
        var msgType: UInt8
        var flags: UInt16
        var seq: UInt32
        var timestampUs: UInt64
        var payloadLen: UInt32

        func encode() -> Data {
            var d = Data(OPCM.magic)
            d.append(OPCM.version)
            d.append(msgType)
            d.appendBE(flags)
            d.appendBE(seq)
            d.appendBE(timestampUs)
            d.appendBE(payloadLen)
            return d
        }

        static func decode(_ d: Data) -> Header? {
            guard d.count >= OPCM.headerLen,
                  Array(d.prefix(4)) == OPCM.magic,
                  d[4] == OPCM.version else { return nil }
            return Header(
                msgType: d[5],
                flags: d.readBE(offset: 6),
                seq: d.readBE(offset: 8),
                timestampUs: d.readBE(offset: 12),
                payloadLen: d.readBE(offset: 20)
            )
        }
    }

    /// Serializa un mensaje completo (cabecera + payload).
    static func frame(_ type: MsgType, seq: UInt32, timestampUs: UInt64, flags: UInt16 = 0, payload: Data = Data()) -> Data {
        let h = Header(msgType: type.rawValue, flags: flags, seq: seq,
                       timestampUs: timestampUs, payloadLen: UInt32(payload.count))
        return h.encode() + payload
    }
}

private extension Data {
    mutating func appendBE<T: FixedWidthInteger>(_ v: T) {
        withUnsafeBytes(of: v.bigEndian) { append(contentsOf: $0) }
    }

    func readBE<T: FixedWidthInteger>(offset: Int) -> T {
        var v: T = 0
        for i in 0..<MemoryLayout<T>.size {
            v = (v << 8) | T(self[startIndex + offset + i])
        }
        return v
    }
}
