import Foundation
import SystemExtensions

/// Activa o desactiva la extensión de cámara con `OSSystemExtensionRequest`.
/// macOS pide permiso al usuario en Ajustes del Sistema la primera vez.
final class ExtensionInstaller: NSObject, ObservableObject, OSSystemExtensionRequestDelegate {
    @Published var status = "Sin cambios."

    func activate() {
        status = "Solicitando la instalación…"
        let request = OSSystemExtensionRequest.activationRequest(
            forExtensionWithIdentifier: CameraConfig.extensionBundleID, queue: .main)
        request.delegate = self
        OSSystemExtensionManager.shared.submitRequest(request)
    }

    func deactivate() {
        status = "Solicitando la desinstalación…"
        let request = OSSystemExtensionRequest.deactivationRequest(
            forExtensionWithIdentifier: CameraConfig.extensionBundleID, queue: .main)
        request.delegate = self
        OSSystemExtensionManager.shared.submitRequest(request)
    }

    func request(
        _ request: OSSystemExtensionRequest,
        actionForReplacingExtension existing: OSSystemExtensionProperties,
        withExtension ext: OSSystemExtensionProperties
    ) -> OSSystemExtensionRequest.ReplacementAction {
        .replace
    }

    func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        status =
            "Falta tu permiso: abre Ajustes del Sistema › General › Ítems de inicio y extensiones "
            + "(o Privacidad y seguridad) y permite la extensión de OpenCam."
    }

    func request(
        _ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result
    ) {
        switch result {
        case .completed:
            status = "Hecho. Si una app de vídeo ya estaba abierta, reiníciala para ver la cámara."
        case .willCompleteAfterReboot:
            status = "Hecho. Se completará al reiniciar el Mac."
        @unknown default:
            status = "Terminado (resultado \(result.rawValue))."
        }
    }

    func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        status = "Error: \(error.localizedDescription)"
    }
}
