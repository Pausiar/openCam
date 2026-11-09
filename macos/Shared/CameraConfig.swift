import CoreVideo
import Foundation

/// Constantes compartidas por la app y la extensión de cámara.
enum CameraConfig {
    /// UID del dispositivo CoreMediaIO; la app lo busca para abrir su stream sink.
    static let deviceUID = "6F4C2A1E-0C3B-4E7A-9D52-0A8E5C1B7F30"
    static let name = "OpenCam"
    static let model = "OpenCam Virtual Camera"
    /// Formato fijo: debe coincidir con `desktop/src/sink_macos.rs`.
    static let width: Int32 = 1920
    static let height: Int32 = 1080
    static let fps: Int32 = 30
    /// YUYV (Y0 Cb Y1 Cr), el mismo orden que escribe el cliente de escritorio.
    static let pixelFormat: OSType = kCVPixelFormatType_422YpCbCr8_yuvs
    /// Identificador de la extensión dentro de la app.
    static let extensionBundleID = "dev.opencam.mac.camera"
}
