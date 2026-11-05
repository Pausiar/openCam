import AVFoundation
import Foundation

/// Une la captura de la cámara trasera, el encoder H.264 y el servidor OPCM.
/// Arranca al recibir CONFIG del escritorio y se detiene al desconectarse.
final class CameraStreamer: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private struct Config: Decodable {
        var codec: String?
        var width: Int?
        var height: Int?
        var fps: Int?
        var bitrate_kbps: Int?
    }

    private let server = OpcmServer()
    private let encoder = H264Encoder()
    private let session = AVCaptureSession()
    /// Cola única: configuración, delegate de captura y estado de abajo. Sin locks.
    private let queue = DispatchQueue(label: "opencam.capture")
    private var sessionConfigured = false
    private var forceKeyframe = false
    private var encoderReady = false
    private var epoch: CMTime?
    private var fps = 30
    private var bitrateKbps = 8000

    func start() throws {
        // Pide el permiso ya, para que el diálogo salga antes de que llegue CONFIG.
        AVCaptureDevice.requestAccess(for: .video) { _ in }
        encoder.onEncoded = { [weak self] data, keyframe, ts in
            self?.server.sendVideo(data, keyframe: keyframe, timestampUs: ts)
        }
        server.capsPayload = Self.capsPayload()
        server.onConfig = { [weak self] payload in self?.queue.async { self?.applyConfig(payload) } }
        server.onStats = { [weak self] payload in self?.queue.async { self?.applyStats(payload) } }
        server.onKeyframeRequest = { [weak self] in self?.queue.async { self?.forceKeyframe = true } }
        server.onClientConnected = { [weak self] in self?.queue.async { self?.forceKeyframe = true } }
        server.onClientDisconnected = { [weak self] in self?.queue.async { self?.stopCapture() } }
        try server.start()
    }

    func stop() {
        server.stop()
        queue.async { [weak self] in self?.stopCapture() }
    }

    // MARK: - Todo lo de abajo corre en `queue`

    private func stopCapture() {
        if session.isRunning { session.stopRunning() }
        encoder.invalidate()
        encoderReady = false
        epoch = nil
    }

    private func applyConfig(_ payload: Data) {
        guard let cfg = try? JSONDecoder().decode(Config.self, from: payload) else {
            server.sendError(code: "bad_config", message: "CONFIG no es JSON válido")
            return
        }
        if let codec = cfg.codec, codec != "h264" {
            server.sendError(code: "unsupported_codec", message: "solo h264")
            return
        }
        guard AVCaptureDevice.authorizationStatus(for: .video) == .authorized else {
            server.sendError(code: "camera_denied", message: "permiso de cámara no concedido")
            return
        }
        guard configureSessionOnce() else {
            server.sendError(code: "camera_unavailable", message: "no se pudo abrir la cámara")
            return
        }

        // Se reconfigura con la captura parada; también vale para un CONFIG repetido.
        stopCapture()
        session.beginConfiguration()
        session.sessionPreset = Self.preset(forWidth: cfg.width ?? 1920, session: session)
        session.commitConfiguration()

        fps = applyFrameRate(cfg.fps ?? 30)
        bitrateKbps = min(max(cfg.bitrate_kbps ?? 8000, 500), 100_000)
        forceKeyframe = true
        // El encoder se crea con el tamaño real del primer frame (ver captureOutput).
        session.startRunning()
    }

    /// Entradas y salida se añaden una sola vez: un segundo addInput falla (canAddInput = false).
    private func configureSessionOnce() -> Bool {
        if sessionConfigured { return true }
        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
              let input = try? AVCaptureDeviceInput(device: device) else { return false }
        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
        output.setSampleBufferDelegate(self, queue: queue)

        session.beginConfiguration()
        defer { session.commitConfiguration() }
        guard session.canAddInput(input), session.canAddOutput(output) else { return false }
        session.addInput(input)
        session.addOutput(output)
        sessionConfigured = true
        return true
    }

    /// El preset más grande que no supere el ancho pedido; si no hay, el más pequeño que el equipo soporte.
    private static func preset(forWidth w: Int, session: AVCaptureSession) -> AVCaptureSession.Preset {
        let table: [(Int, AVCaptureSession.Preset)] = [
            (3840, .hd4K3840x2160), (1920, .hd1920x1080), (1280, .hd1280x720)]
        for (width, p) in table where width <= w && session.canSetSessionPreset(p) { return p }
        return table.reversed().first { session.canSetSessionPreset($0.1) }?.1 ?? .high
    }

    /// Fija los fps en el dispositivo si el formato activo los admite; devuelve los fps efectivos.
    private func applyFrameRate(_ wanted: Int) -> Int {
        guard let device = (session.inputs.first as? AVCaptureDeviceInput)?.device,
              device.activeFormat.videoSupportedFrameRateRanges.contains(where: {
                  Double(wanted) >= $0.minFrameRate && Double(wanted) <= $0.maxFrameRate }),
              (try? device.lockForConfiguration()) != nil else {
            return 30
        }
        let d = CMTime(value: 1, timescale: CMTimeScale(wanted))
        device.activeVideoMinFrameDuration = d
        device.activeVideoMaxFrameDuration = d
        device.unlockForConfiguration()
        return wanted
    }

    /// STATS: ajusta el bitrate objetivo del encoder sin reiniciarlo.
    private func applyStats(_ payload: Data) {
        guard let obj = try? JSONSerialization.jsonObject(with: payload) as? [String: Any],
              let kbps = (obj["target_bitrate_kbps"] as? NSNumber)?.intValue, kbps > 0 else { return }
        bitrateKbps = min(max(kbps, 500), 100_000)
        encoder.setBitrate(kbps: bitrateKbps)
    }

    /// CAPS según el hardware: 4K solo si la cámara trasera lo soporta.
    private static func capsPayload() -> Data {
        var res = [[1280, 720], [1920, 1080]]
        if let d = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
           d.supportsSessionPreset(.hd4K3840x2160) { res.append([3840, 2160]) }
        let caps: [String: Any] = ["platform": "ios", "codecs": ["h264"], "resolutions": res,
                                   "fps": [24, 30, 60], "lenses": ["wide"]]
        return (try? JSONSerialization.data(withJSONObject: caps)) ?? Data()
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        if !encoderReady {
            let w = Int32(CVPixelBufferGetWidth(pixelBuffer)), h = Int32(CVPixelBufferGetHeight(pixelBuffer))
            guard (try? encoder.configure(width: w, height: h, fps: fps, bitrateKbps: bitrateKbps)) != nil else { return }
            encoderReady = true
        }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        if epoch == nil { epoch = pts }
        let elapsed = CMTimeSubtract(pts, epoch ?? pts)
        let ts = UInt64(max(0, CMTimeGetSeconds(elapsed)) * 1_000_000)
        let keyframe = forceKeyframe
        forceKeyframe = false
        encoder.encode(pixelBuffer, timestampUs: ts, forceKeyframe: keyframe)
    }
}
