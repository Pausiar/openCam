import Foundation
import Darwin

/// Ajustes de vídeo elegidos en el iPhone, guardados en UserDefaults.
struct StreamSettings: Equatable {
    var width = 1920
    var fps = 30
    var bitrateKbps = 8000

    var height: Int { width * 9 / 16 }

    static let fpsOptions = [24, 30, 60]
    struct BitrateOption: Hashable {
        let label: String
        let kbps: Int
    }

    static let bitrateOptions = [
        BitrateOption(label: "Baja (2 Mbps)", kbps: 2000), BitrateOption(label: "Media (4 Mbps)", kbps: 4000),
        BitrateOption(label: "Alta (8 Mbps)", kbps: 8000), BitrateOption(label: "Muy alta (12 Mbps)", kbps: 12000),
        BitrateOption(label: "Máxima (20 Mbps)", kbps: 20000)]

    static func load() -> StreamSettings {
        let d = UserDefaults.standard
        var s = StreamSettings()
        if d.integer(forKey: "width") > 0 { s.width = d.integer(forKey: "width") }
        if d.integer(forKey: "fps") > 0 { s.fps = d.integer(forKey: "fps") }
        if d.integer(forKey: "bitrate_kbps") > 0 { s.bitrateKbps = d.integer(forKey: "bitrate_kbps") }
        return s
    }

    func save() {
        let d = UserDefaults.standard
        d.set(width, forKey: "width")
        d.set(fps, forKey: "fps")
        d.set(bitrateKbps, forKey: "bitrate_kbps")
    }
}

/// IPv4 por las que el PC puede llegar al iPhone (Wi-Fi `en0`, punto de acceso `bridge100`).
struct NetAddress: Hashable {
    let ip: String
    let kind: String
}

enum NetInfo {
    static func addresses() -> [NetAddress] {
        var out: [NetAddress] = []
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0, let first = ifaddr else { return [] }
        defer { freeifaddrs(ifaddr) }
        for ptr in sequence(first: first, next: { $0.pointee.ifa_next }) {
            let ifa = ptr.pointee
            guard let addr = ifa.ifa_addr, addr.pointee.sa_family == UInt8(AF_INET) else { continue }
            let name = String(cString: ifa.ifa_name)
            let kind: String
            if name == "en0" { kind = "Wi-Fi" } else if name.hasPrefix("bridge") { kind = "Punto de acceso" } else { continue }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(addr, socklen_t(addr.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                out.append(NetAddress(ip: String(cString: host), kind: kind))
            }
        }
        return out
    }
}
