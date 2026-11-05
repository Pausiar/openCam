import SwiftUI
import UIKit

@main
struct OpenCamApp: App {
    private let streamer = CameraStreamer()

    var body: some Scene {
        WindowGroup {
            ContentView(streamer: streamer)
        }
    }
}

struct ContentView: View {
    let streamer: CameraStreamer
    @State private var running = false
    @State private var error: String?

    var body: some View {
        VStack(spacing: 16) {
            Text("OpenCam").font(.largeTitle).bold()
            Text(running ? "Esperando al escritorio en el puerto 4747" : "Parado")
                .foregroundStyle(.secondary)
            if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            Button(running ? "Parar" : "Iniciar") {
                running ? stop() : start()
            }
            .buttonStyle(.borderedProminent)
        }
        .padding()
    }

    private func start() {
        do {
            try streamer.start()
            UIApplication.shared.isIdleTimerDisabled = true // la pantalla no debe bloquearse mientras se emite
            running = true
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func stop() {
        streamer.stop()
        UIApplication.shared.isIdleTimerDisabled = false
        running = false
    }
}
