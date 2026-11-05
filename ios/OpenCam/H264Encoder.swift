import AVFoundation
import VideoToolbox
import CoreMedia

/// Codifica H.264 con VideoToolbox y entrega access units en formato Annex-B (start codes),
/// que es lo que espera el decodificador del escritorio.
final class H264Encoder {
    var onEncoded: ((Data, _ keyframe: Bool, _ timestampUs: UInt64) -> Void)?

    private var session: VTCompressionSession?
    private var width: Int32 = 0
    private var height: Int32 = 0

    func configure(width: Int32, height: Int32, fps: Int, bitrateKbps: Int) throws {
        self.width = width
        self.height = height
        var s: VTCompressionSession?
        let status = VTCompressionSessionCreate(
            allocator: nil, width: width, height: height,
            codecType: kCMVideoCodecType_H264, encoderSpecification: nil,
            imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &s)
        guard status == noErr, let session = s else { throw NSError(domain: "opencam", code: Int(status)) }
        // Baja latencia: sin reordenamiento de B-frames y con keyframes periódicos.
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse)
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_ProfileLevel,
                             value: kVTProfileLevel_H264_High_AutoLevel)
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AverageBitRate,
                             value: NSNumber(value: bitrateKbps * 1000))
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_ExpectedFrameRate,
                             value: NSNumber(value: fps))
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_MaxKeyFrameInterval,
                             value: NSNumber(value: fps))
        VTCompressionSessionPrepareToEncodeFrames(session)
        self.session = session
    }

    func encode(_ pixelBuffer: CVPixelBuffer, timestampUs: UInt64, forceKeyframe: Bool) {
        guard let session else { return }
        let pts = CMTime(value: CMTimeValue(timestampUs), timescale: 1_000_000)
        let props: CFDictionary? = forceKeyframe
            ? [kVTEncodeFrameOptionKey_ForceKeyFrame: kCFBooleanTrue] as CFDictionary
            : nil
        VTCompressionSessionEncodeFrame(session, imageBuffer: pixelBuffer, presentationTimeStamp: pts,
                                        duration: .invalid, frameProperties: props,
                                        infoFlags: nil) { [weak self] status, _, sample in
            guard status == noErr, let sample else { return }
            self?.emit(sample, timestampUs: timestampUs)
        }
    }

    func invalidate() {
        if let session { VTCompressionSessionInvalidate(session) }
        session = nil
    }

    /// Cambia el bitrate objetivo en caliente (STATS). Llamar desde la cola de captura.
    func setBitrate(kbps: Int) {
        guard let session else { return }
        VTSessionSetProperty(session, key: kVTCompressionPropertyKey_AverageBitRate,
                             value: NSNumber(value: kbps * 1000))
    }

    /// Convierte un CMSampleBuffer AVCC a Annex-B. Los SPS/PPS van delante de los keyframes.
    private func emit(_ sample: CMSampleBuffer, timestampUs: UInt64) {
        guard let block = CMSampleBufferGetDataBuffer(sample) else { return }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let notSync = attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false
        let keyframe = !notSync

        var out = Data()
        var nalHeaderLen: Int32 = 4
        if let fmt = CMSampleBufferGetFormatDescription(sample) {
            var count = 0
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                fmt, parameterSetIndex: 0, parameterSetPointerOut: nil,
                parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &nalHeaderLen)
            if keyframe {
                for i in 0..<count {
                    var ptr: UnsafePointer<UInt8>?
                    var size = 0
                    CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                        fmt, parameterSetIndex: i, parameterSetPointerOut: &ptr,
                        parameterSetSizeOut: &size, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil)
                    if let ptr {
                        out.append(contentsOf: [0, 0, 0, 1])
                        out.append(ptr, count: size)
                    }
                }
            }
        }

        // Copia a un Data contiguo: el CMBlockBuffer puede venir fragmentado y
        // CMBlockBufferGetDataPointer solo da el primer tramo.
        let length = CMBlockBufferGetDataLength(block)
        var avcc = Data(count: length)
        let copied = avcc.withUnsafeMutableBytes { raw -> OSStatus in
            guard let base = raw.baseAddress else { return -1 }
            return CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: base)
        }
        guard copied == noErr else { return }

        // Cada NAL viene precedido de su longitud (big-endian); se cambia por start code.
        let n = Int(nalHeaderLen)
        var offset = 0
        while offset + n <= length {
            var nalLen = 0
            for k in 0..<n { nalLen = (nalLen << 8) | Int(avcc[offset + k]) }
            offset += n
            guard nalLen > 0, offset + nalLen <= length else { break } // NAL truncado: se descarta el resto
            out.append(contentsOf: [0, 0, 0, 1])
            out.append(avcc[offset..<offset + nalLen])
            offset += nalLen
        }
        onEncoded?(out, keyframe, timestampUs)
    }
}
