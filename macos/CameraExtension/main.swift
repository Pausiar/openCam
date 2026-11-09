import CoreMediaIO
import Foundation

// Punto de entrada de la extensión de cámara: macOS la arranca como servicio aparte.
let providerSource = ProviderSource(clientQueue: nil)
CMIOExtensionProvider.startService(provider: providerSource.provider)
CFRunLoopRun()
