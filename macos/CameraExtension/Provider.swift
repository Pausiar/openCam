import CoreMediaIO
import Foundation
import IOKit.audio
import os.log

private let log = Logger(subsystem: "dev.opencam.mac.camera", category: "camera")

/// Pasado este tiempo sin frames del cliente, la cámara muestra negro.
private let staleAfterNs: UInt64 = 2_000_000_000

// MARK: - Dispositivo

/// Cámara virtual con dos streams:
/// - `source`: lo que ven las apps de vídeo (Zoom, Meet, FaceTime...).
/// - `sink`: la entrada; la app OpenCam (`--feed`) mete ahí los frames del móvil.
/// Un temporizador a `CameraConfig.fps` reenvía al `source` el último frame recibido,
/// o negro si el cliente no envía nada.
final class DeviceSource: NSObject, CMIOExtensionDeviceSource {
    private(set) var device: CMIOExtensionDevice!
    private var sourceStream: SourceStream!
    private var sinkStream: SinkStream!

    private var formatDescription: CMFormatDescription!
    private var blackFrame: CVPixelBuffer?

    private let queue = DispatchQueue(label: "dev.opencam.mac.camera.timer", qos: .userInteractive)
    private var timer: DispatchSourceTimer?
    private var sourceClients = 0

    // Solo se tocan en `queue`.
    private var lastFrame: CVPixelBuffer?
    private var lastFrameNs: UInt64 = 0

    init(localizedName: String) {
        super.init()
        let deviceID = UUID(uuidString: CameraConfig.deviceUID)!
        device = CMIOExtensionDevice(
            localizedName: localizedName, deviceID: deviceID, legacyDeviceID: CameraConfig.deviceUID,
            source: self)

        CMVideoFormatDescriptionCreate(
            allocator: kCFAllocatorDefault, codecType: CameraConfig.pixelFormat,
            width: CameraConfig.width, height: CameraConfig.height, extensions: nil,
            formatDescriptionOut: &formatDescription)
        blackFrame = Self.makeBlackFrame()

        let frameDuration = CMTime(value: 1, timescale: CameraConfig.fps)
        let format = CMIOExtensionStreamFormat(
            formatDescription: formatDescription, maxFrameDuration: frameDuration,
            minFrameDuration: frameDuration, validFrameDurations: nil)

        sourceStream = SourceStream(
            localizedName: "OpenCam.Video", streamID: UUID(), format: format, deviceSource: self)
        sinkStream = SinkStream(
            localizedName: "OpenCam.Video.Sink", streamID: UUID(), format: format, deviceSource: self)
        do {
            try device.addStream(sourceStream.stream)
            try device.addStream(sinkStream.stream)
        } catch {
            fatalError("No se pudieron añadir los streams: \(error.localizedDescription)")
        }
    }

    var availableProperties: Set<CMIOExtensionProperty> { [.deviceTransportType, .deviceModel] }

    func deviceProperties(forProperties properties: Set<CMIOExtensionProperty>) throws
        -> CMIOExtensionDeviceProperties
    {
        let props = CMIOExtensionDeviceProperties(dictionary: [:])
        if properties.contains(.deviceTransportType) {
            props.transportType = kIOAudioDeviceTransportTypeVirtual
        }
        if properties.contains(.deviceModel) {
            props.model = CameraConfig.model
        }
        return props
    }

    func setDeviceProperties(_ deviceProperties: CMIOExtensionDeviceProperties) throws {}

    // MARK: Stream de salida (apps de vídeo)

    func startSource() {
        queue.async { [self] in
            sourceClients += 1
            guard timer == nil else { return }
            let t = DispatchSource.makeTimerSource(flags: .strict, queue: queue)
            let interval = 1.0 / Double(CameraConfig.fps)
            t.schedule(deadline: .now(), repeating: interval, leeway: .milliseconds(1))
            t.setEventHandler { [weak self] in self?.tick() }
            t.resume()
            timer = t
        }
    }

    func stopSource() {
        queue.async { [self] in
            sourceClients = max(0, sourceClients - 1)
            if sourceClients == 0 {
                timer?.cancel()
                timer = nil
            }
        }
    }

    private func tick() {
        let now = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
        let fresh = lastFrame != nil && now &- lastFrameNs < staleAfterNs
        guard let pixelBuffer = fresh ? lastFrame : blackFrame else { return }

        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: CameraConfig.fps),
            presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()),
            decodeTimeStamp: .invalid)
        var sampleBuffer: CMSampleBuffer?
        let err = CMSampleBufferCreateForImageBuffer(
            allocator: kCFAllocatorDefault, imageBuffer: pixelBuffer, dataReady: true,
            makeDataReadyCallback: nil, refcon: nil, formatDescription: formatDescription,
            sampleTiming: &timing, sampleBufferOut: &sampleBuffer)
        guard err == noErr, let sampleBuffer else {
            log.error("CMSampleBufferCreateForImageBuffer: \(err)")
            return
        }
        sourceStream.stream.send(
            sampleBuffer, discontinuity: [],
            hostTimeInNanoseconds: UInt64(timing.presentationTimeStamp.seconds * Double(NSEC_PER_SEC)))
    }

    // MARK: Stream de entrada (app OpenCam)

    func startSink(client: CMIOExtensionClient) {
        consume(from: client)
    }

    private func consume(from client: CMIOExtensionClient) {
        guard sinkStream.isRunning else { return }
        sinkStream.stream.consumeSampleBuffer(from: client) {
            [weak self] sampleBuffer, sequenceNumber, _, hasMore, error in
            guard let self else { return }
            if let sampleBuffer, let image = CMSampleBufferGetImageBuffer(sampleBuffer) {
                let now = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
                self.queue.async {
                    self.lastFrame = image
                    self.lastFrameNs = now
                }
                let output = CMIOExtensionScheduledOutput(
                    sequenceNumber: sequenceNumber,
                    hostTimeInNanoseconds: UInt64(
                        CMClockGetTime(CMClockGetHostTimeClock()).seconds * Double(NSEC_PER_SEC)))
                self.sinkStream.stream.notifyScheduledOutputChanged(output)
            } else if let error {
                log.debug("consumeSampleBuffer: \(error.localizedDescription)")
            }
            // Sin frames pendientes se espera un poco para no girar en vacío.
            let delay: DispatchTimeInterval = hasMore ? .nanoseconds(0) : .milliseconds(5)
            self.queue.asyncAfter(deadline: .now() + delay) { self.consume(from: client) }
        }
    }

    // MARK: Frame negro

    private static func makeBlackFrame() -> CVPixelBuffer? {
        let attrs: [CFString: Any] = [kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any]]
        var pb: CVPixelBuffer?
        CVPixelBufferCreate(
            kCFAllocatorDefault, Int(CameraConfig.width), Int(CameraConfig.height),
            CameraConfig.pixelFormat, attrs as CFDictionary, &pb)
        guard let pb else { return nil }
        CVPixelBufferLockBaseAddress(pb, [])
        if let base = CVPixelBufferGetBaseAddress(pb) {
            let rowBytes = CVPixelBufferGetBytesPerRow(pb)
            let height = CVPixelBufferGetHeight(pb)
            // YUYV negro: Y=16, U=V=128.
            let p = base.assumingMemoryBound(to: UInt8.self)
            for i in stride(from: 0, to: rowBytes * height - 1, by: 2) {
                p[i] = 16
                p[i + 1] = 128
            }
        }
        CVPixelBufferUnlockBaseAddress(pb, [])
        return pb
    }
}

// MARK: - Streams

final class SourceStream: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    private let format: CMIOExtensionStreamFormat
    private weak var deviceSource: DeviceSource?

    init(localizedName: String, streamID: UUID, format: CMIOExtensionStreamFormat, deviceSource: DeviceSource) {
        self.format = format
        self.deviceSource = deviceSource
        super.init()
        stream = CMIOExtensionStream(
            localizedName: localizedName, streamID: streamID, direction: .source,
            clockType: .hostTime, source: self)
    }

    var formats: [CMIOExtensionStreamFormat] { [format] }

    var availableProperties: Set<CMIOExtensionProperty> { [.streamActiveFormatIndex, .streamFrameDuration] }

    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws
        -> CMIOExtensionStreamProperties
    {
        let props = CMIOExtensionStreamProperties(dictionary: [:])
        if properties.contains(.streamActiveFormatIndex) { props.activeFormatIndex = 0 }
        if properties.contains(.streamFrameDuration) {
            props.frameDuration = CMTime(value: 1, timescale: CameraConfig.fps)
        }
        return props
    }

    func setStreamProperties(_ streamProperties: CMIOExtensionStreamProperties) throws {}

    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool { true }

    func startStream() throws { deviceSource?.startSource() }

    func stopStream() throws { deviceSource?.stopSource() }
}

final class SinkStream: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    private let format: CMIOExtensionStreamFormat
    private weak var deviceSource: DeviceSource?
    private var client: CMIOExtensionClient?
    private let lock = NSLock()
    private var running = false

    var isRunning: Bool {
        lock.lock()
        defer { lock.unlock() }
        return running
    }

    init(localizedName: String, streamID: UUID, format: CMIOExtensionStreamFormat, deviceSource: DeviceSource) {
        self.format = format
        self.deviceSource = deviceSource
        super.init()
        stream = CMIOExtensionStream(
            localizedName: localizedName, streamID: streamID, direction: .sink,
            clockType: .hostTime, source: self)
    }

    var formats: [CMIOExtensionStreamFormat] { [format] }

    var availableProperties: Set<CMIOExtensionProperty> {
        [
            .streamActiveFormatIndex, .streamFrameDuration, .streamSinkBufferQueueSize,
            .streamSinkBuffersRequiredForStartup,
        ]
    }

    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws
        -> CMIOExtensionStreamProperties
    {
        let props = CMIOExtensionStreamProperties(dictionary: [:])
        if properties.contains(.streamActiveFormatIndex) { props.activeFormatIndex = 0 }
        if properties.contains(.streamFrameDuration) {
            props.frameDuration = CMTime(value: 1, timescale: CameraConfig.fps)
        }
        if properties.contains(.streamSinkBufferQueueSize) { props.sinkBufferQueueSize = 1 }
        if properties.contains(.streamSinkBuffersRequiredForStartup) {
            props.sinkBuffersRequiredForStartup = 1
        }
        return props
    }

    func setStreamProperties(_ streamProperties: CMIOExtensionStreamProperties) throws {}

    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool {
        self.client = client
        return true
    }

    func startStream() throws {
        guard let client else { return }
        lock.lock()
        running = true
        lock.unlock()
        deviceSource?.startSink(client: client)
    }

    func stopStream() throws {
        lock.lock()
        running = false
        lock.unlock()
    }
}

// MARK: - Proveedor

final class ProviderSource: NSObject, CMIOExtensionProviderSource {
    private(set) var provider: CMIOExtensionProvider!
    private var deviceSource: DeviceSource!

    init(clientQueue: DispatchQueue?) {
        super.init()
        provider = CMIOExtensionProvider(source: self, clientQueue: clientQueue)
        deviceSource = DeviceSource(localizedName: CameraConfig.name)
        do {
            try provider.addDevice(deviceSource.device)
        } catch {
            fatalError("No se pudo añadir el dispositivo: \(error.localizedDescription)")
        }
    }

    func connect(to client: CMIOExtensionClient) throws {}

    func disconnect(from client: CMIOExtensionClient) {}

    var availableProperties: Set<CMIOExtensionProperty> { [.providerManufacturer] }

    func providerProperties(forProperties properties: Set<CMIOExtensionProperty>) throws
        -> CMIOExtensionProviderProperties
    {
        let props = CMIOExtensionProviderProperties(dictionary: [:])
        if properties.contains(.providerManufacturer) { props.manufacturer = "OpenCam" }
        return props
    }

    func setProviderProperties(_ providerProperties: CMIOExtensionProviderProperties) throws {}
}
