import Foundation

/// Dos modos en el mismo ejecutable:
/// - Sin argumentos: ventana para instalar o quitar la cámara virtual.
/// - `--feed [--fps N]`: lee frames YUYV de stdin y los envía a la cámara virtual.
///   Lo lanza el cliente de escritorio (`opencam-desktop`, sink `macos`).
@main
enum Main {
    static func main() {
        let args = CommandLine.arguments
        if args.contains("--feed") {
            var fps = CameraConfig.fps
            if let i = args.firstIndex(of: "--fps"), i + 1 < args.count, let n = Int32(args[i + 1]), n > 0 {
                fps = n
            }
            exit(Feeder.run(fps: fps))
        }
        OpenCamApp.main()
    }
}
