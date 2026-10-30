import AVFoundation
import Foundation

/// Une la captura de la cámara trasera, el encoder H.264 y el servidor OPCM.
/// Arranca al recibir CONFIG del escritorio y se detiene al desconectarse.
final class CameraStreamer: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let server = OpcmServer()
    private let encoder = H264Encoder()
    private let session = AVCaptureSession()
    private let outputQueue = DispatchQueue(label: "opencam.capture")
    private var forceKeyframe = false
    private var epoch: CMTime?

    func start() throws {
        encoder.onEncoded = { [weak self] data, keyframe, ts in
            self?.server.sendVideo(data, keyframe: keyframe, timestampUs: ts)
        }
        server.onConfig = { [weak self] _ in self?.startCapture() }
        server.onKeyframeRequest = { [weak self] in self?.forceKeyframe = true }
        try server.start()
    }

    func stop() {
        session.stopRunning()
        encoder.invalidate()
        server.stop()
    }

    private func startCapture() {
        guard !session.isRunning else { return }
        // El escritorio pide 1080p; se usa el preset más cercano disponible.
        session.beginConfiguration()
        session.sessionPreset = .hd1920x1080
        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else { session.commitConfiguration(); return }
        session.addInput(input)

        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
        output.setSampleBufferDelegate(self, queue: outputQueue)
        guard session.canAddOutput(output) else { session.commitConfiguration(); return }
        session.addOutput(output)
        session.commitConfiguration()

        try? encoder.configure(width: 1920, height: 1080, fps: 30, bitrateKbps: 8000)
        session.startRunning()
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        if epoch == nil { epoch = pts }
        let elapsed = CMTimeSubtract(pts, epoch ?? pts)
        let ts = UInt64(CMTimeGetSeconds(elapsed) * 1_000_000)
        let keyframe = forceKeyframe
        forceKeyframe = false
        encoder.encode(pixelBuffer, timestampUs: ts, forceKeyframe: keyframe)
    }
}
