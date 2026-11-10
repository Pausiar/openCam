import AVFoundation
import SwiftUI
import UIKit

/// Paleta lila de OpenCam (la misma que el escritorio y Android).
enum Theme {
    static let accent = Color(red: 108 / 255, green: 92 / 255, blue: 231 / 255)
    static let accentDark = Color(red: 84 / 255, green: 68 / 255, blue: 205 / 255)
    static let accentSoft = Color(red: 236 / 255, green: 232 / 255, blue: 253 / 255)
    static let background = Color(red: 246 / 255, green: 244 / 255, blue: 253 / 255)
    static let border = Color(red: 229 / 255, green: 225 / 255, blue: 245 / 255)
    static let text = Color(red: 31 / 255, green: 27 / 255, blue: 58 / 255)
    static let muted = Color(red: 107 / 255, green: 103 / 255, blue: 136 / 255)
    static let green = Color(red: 34 / 255, green: 197 / 255, blue: 94 / 255)
    static let preview = Color(red: 30 / 255, green: 27 / 255, blue: 46 / 255)
}

/// Tarjeta blanca con borde suave y sombra.
struct CardModifier: ViewModifier {
    var padding: CGFloat = 16

    func body(content: Content) -> some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(Color.white))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).stroke(Theme.border, lineWidth: 1))
            .shadow(color: Theme.accent.opacity(0.08), radius: 10, x: 0, y: 4)
    }
}

extension View {
    func card(padding: CGFloat = 16) -> some View { modifier(CardModifier(padding: padding)) }
}

/// Logotipo: cuadrado morado con un objetivo de cámara.
struct LogoView: View {
    var size: CGFloat = 56

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
                .fill(LinearGradient(colors: [Color(red: 0.44, green: 0.41, blue: 0.94),
                                              Color(red: 0.31, green: 0.27, blue: 0.75)],
                                     startPoint: .top, endPoint: .bottom))
            Circle().fill(Color(red: 0.91, green: 0.93, blue: 1)).frame(width: size * 0.68)
            Circle().fill(Color(red: 0.11, green: 0.14, blue: 0.31)).frame(width: size * 0.52)
            Circle().fill(Color(red: 0.23, green: 0.43, blue: 0.9)).frame(width: size * 0.34)
            Circle().fill(Color(red: 0.07, green: 0.09, blue: 0.2)).frame(width: size * 0.16)
            Circle().fill(Color.white.opacity(0.85)).frame(width: size * 0.09)
                .offset(x: -size * 0.07, y: -size * 0.07)
            Circle().fill(Color(red: 1, green: 0.48, blue: 0.35)).frame(width: size * 0.14)
                .offset(x: size * 0.3, y: -size * 0.3)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// Píldora con un punto de color (estado del servidor).
struct StatusPill: View {
    let color: Color
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Circle().fill(color).frame(width: 10, height: 10)
            Text(text).font(.subheadline.weight(.semibold)).foregroundColor(Theme.text)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .background(Capsule().fill(Color.white))
        .overlay(Capsule().stroke(Theme.border, lineWidth: 1))
        .shadow(color: Theme.accent.opacity(0.08), radius: 8, x: 0, y: 3)
    }
}

/// Etiqueta translúcida sobre la vista previa.
struct Chip: View {
    var icon: String?
    let text: String

    var body: some View {
        HStack(spacing: 6) {
            if let icon = icon { Image(systemName: icon) }
            Text(text)
        }
        .font(.footnote.weight(.medium))
        .foregroundColor(.white)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color.black.opacity(0.55)))
    }
}

/// Vista previa de la sesión de captura (lo que recibe el PC).
struct CameraPreview: UIViewRepresentable {
    let session: AVCaptureSession

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.backgroundColor = .clear
        view.previewLayer.session = session
        view.previewLayer.videoGravity = .resizeAspectFill
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {
        // El PC recibe la imagen en horizontal (orientación del sensor): se muestra igual.
        if let c = view.previewLayer.connection, c.isVideoOrientationSupported {
            c.videoOrientation = .landscapeRight
        }
    }
}
