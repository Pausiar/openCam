import SwiftUI

struct OpenCamApp: App {
    @StateObject private var installer = ExtensionInstaller()

    var body: some Scene {
        WindowGroup("OpenCam") {
            ContentView()
                .environmentObject(installer)
                .frame(minWidth: 460, minHeight: 300)
        }
        .windowResizability(.contentSize)
    }
}

struct ContentView: View {
    @EnvironmentObject private var installer: ExtensionInstaller

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Cámara virtual OpenCam")
                .font(.title2.bold())
            Text(
                "Instala la cámara una vez. Después arranca el cliente `opencam-desktop` con la "
                    + "dirección del móvil y elige «OpenCam» como cámara en tu app de vídeo."
            )
            .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("Instalar cámara") { installer.activate() }
                    .keyboardShortcut(.defaultAction)
                Button("Desinstalar") { installer.deactivate() }
            }

            Text(installer.status)
                .font(.callout)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)

            Spacer()
            Text("La app debe estar en la carpeta Aplicaciones para poder instalar la cámara.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .padding(24)
    }
}
