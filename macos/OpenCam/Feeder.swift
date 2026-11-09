import CoreMedia
import CoreMediaIO
import CoreVideo
import Foundation

/// Modo `--feed`: lee de stdin frames YUYV de `CameraConfig.width` x `CameraConfig.height`
/// (sin cabeceras) y los mete en el stream sink de la cámara virtual OpenCam.
/// Termina con código 0 cuando stdin se cierra (el cliente de escritorio se detuvo).
enum Feeder {
    static func run(fps: Int32) -> Int32 {
        let width = Int(CameraConfig.width)
        let height = Int(CameraConfig.height)
        let frameBytes = width * height * 2

        guard let device = findDevice(uid: CameraConfig.deviceUID) else {
            fail(
                "no está la cámara virtual OpenCam. Abre OpenCam.app desde Aplicaciones, pulsa "
                    + "«Instalar cámara» y acepta el permiso en Ajustes del Sistema.")
            return 1
        }
        guard let stream = sinkStream(of: device) else {
            fail("la cámara OpenCam no tiene stream de entrada; reinstálala desde OpenCam.app.")
            return 1
        }

        var queueRef: Unmanaged<CMSimpleQueue>?
        var err = CMIOStreamCopyBufferQueue(stream, { _, _, _ in }, nil, &queueRef)
        guard err == noErr, let queue = queueRef?.takeRetainedValue() else {
            fail("CMIOStreamCopyBufferQueue devolvió \(err).")
            return 1
        }
        err = CMIODeviceStartStream(device, stream)
        guard err == noErr else {
            fail("CMIODeviceStartStream devolvió \(err).")
            return 1
        }
        defer { CMIODeviceStopStream(device, stream) }

        var format: CMFormatDescription?
        CMVideoFormatDescriptionCreate(
            allocator: kCFAllocatorDefault, codecType: CameraConfig.pixelFormat,
            width: CameraConfig.width, height: CameraConfig.height, extensions: nil,
            formatDescriptionOut: &format)
        var pool: CVPixelBufferPool?
        let attrs: [CFString: Any] = [
            kCVPixelBufferWidthKey: width,
            kCVPixelBufferHeightKey: height,
            kCVPixelBufferPixelFormatTypeKey: CameraConfig.pixelFormat,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any],
        ]
        CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attrs as CFDictionary, &pool)
        guard let format, let pool else {
            fail("no se pudo preparar el formato de vídeo.")
            return 1
        }

        var frame = [UInt8](repeating: 0, count: frameBytes)
        let duration = CMTime(value: 1, timescale: fps)
        while readExactly(&frame) {
            var pbOut: CVPixelBuffer?
            guard CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &pbOut) == kCVReturnSuccess,
                let pb = pbOut
            else { continue }
            copy(frame, into: pb, rowBytes: width * 2, rows: height)

            var timing = CMSampleTimingInfo(
                duration: duration, presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()),
                decodeTimeStamp: .invalid)
            var sbOut: CMSampleBuffer?
            guard
                CMSampleBufferCreateForImageBuffer(
                    allocator: kCFAllocatorDefault, imageBuffer: pb, dataReady: true,
                    makeDataReadyCallback: nil, refcon: nil, formatDescription: format,
                    sampleTiming: &timing, sampleBufferOut: &sbOut) == noErr,
                let sb = sbOut
            else { continue }

            // Cola llena: la extensión va atrasada; se descarta el frame para no acumular latencia.
            if CMSimpleQueueGetCount(queue) >= CMSimpleQueueGetCapacity(queue) { continue }
            let ptr = Unmanaged.passRetained(sb).toOpaque()
            if CMSimpleQueueEnqueue(queue, element: ptr) != noErr {
                Unmanaged<CMSampleBuffer>.fromOpaque(ptr).release()
            }
        }
        return 0
    }

    // MARK: - CoreMediaIO

    private static func address(_ selector: Int) -> CMIOObjectPropertyAddress {
        CMIOObjectPropertyAddress(
            mSelector: CMIOObjectPropertySelector(selector),
            mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
            mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
    }

    /// Lee una propiedad que es una lista de IDs de objeto.
    private static func objectIDs(of object: CMIOObjectID, selector: Int) -> [CMIOObjectID] {
        var addr = address(selector)
        var size: UInt32 = 0
        guard CMIOObjectGetPropertyDataSize(object, &addr, 0, nil, &size) == noErr, size > 0 else {
            return []
        }
        var ids = [CMIOObjectID](repeating: 0, count: Int(size) / MemoryLayout<CMIOObjectID>.size)
        var used: UInt32 = 0
        guard CMIOObjectGetPropertyData(object, &addr, 0, nil, size, &used, &ids) == noErr else {
            return []
        }
        return Array(ids.prefix(Int(used) / MemoryLayout<CMIOObjectID>.size))
    }

    private static func findDevice(uid: String) -> CMIOObjectID? {
        // Los dispositivos de extensiones pueden tardar un momento en aparecer.
        for _ in 0..<20 {
            for device in objectIDs(of: CMIOObjectID(kCMIOObjectSystemObject), selector: Int(kCMIOHardwarePropertyDevices)) {
                var addr = address(Int(kCMIODevicePropertyDeviceUID))
                var value: Unmanaged<CFString>?
                var used: UInt32 = 0
                let size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
                guard CMIOObjectGetPropertyData(device, &addr, 0, nil, size, &used, &value) == noErr,
                    let str = value?.takeRetainedValue()
                else { continue }
                if (str as String) == uid { return device }
            }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return nil
    }

    /// El stream de entrada (dirección 1) del dispositivo; la extensión lo añade en segundo lugar.
    private static func sinkStream(of device: CMIOObjectID) -> CMIOStreamID? {
        let streams = objectIDs(of: device, selector: Int(kCMIODevicePropertyStreams))
        for stream in streams {
            var addr = address(Int(kCMIOStreamPropertyDirection))
            var direction: UInt32 = 0
            var used: UInt32 = 0
            if CMIOObjectGetPropertyData(
                stream, &addr, 0, nil, UInt32(MemoryLayout<UInt32>.size), &used, &direction) == noErr,
                direction == 1
            {
                return stream
            }
        }
        return streams.count > 1 ? streams[1] : nil
    }

    // MARK: - E/S

    private static func readExactly(_ buffer: inout [UInt8]) -> Bool {
        var filled = 0
        let total = buffer.count
        return buffer.withUnsafeMutableBytes { raw in
            while filled < total {
                let n = read(STDIN_FILENO, raw.baseAddress! + filled, total - filled)
                if n > 0 {
                    filled += n
                } else if n < 0 && errno == EINTR {
                    continue
                } else {
                    return false
                }
            }
            return true
        }
    }

    private static func copy(_ frame: [UInt8], into pb: CVPixelBuffer, rowBytes: Int, rows: Int) {
        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }
        guard let base = CVPixelBufferGetBaseAddress(pb) else { return }
        let dstStride = CVPixelBufferGetBytesPerRow(pb)
        frame.withUnsafeBytes { src in
            for row in 0..<rows {
                memcpy(base + row * dstStride, src.baseAddress! + row * rowBytes, rowBytes)
            }
        }
    }

    private static func fail(_ message: String) {
        FileHandle.standardError.write("OpenCam (cámara virtual): \(message)\n".data(using: .utf8)!)
    }
}
