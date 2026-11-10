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
    @State private var connected = false
    @State private var error: String?
    @State private var code: String?
    @State private var settings = StreamSettings.load()
    @State private var addresses: [NetAddress] = []
    private let resolutions = CameraStreamer.supportedResolutions().map { $0[0] }

    var body: some View {
        NavigationView {
            Form {
                Section {
                    HStack {
                        Circle().fill(statusColor).frame(width: 10, height: 10)
                        Text(statusText)
                    }
                    if running, !connected, let code {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Código de emparejamiento (escríbelo en el PC la primera vez)")
                                .font(.footnote).foregroundStyle(.secondary)
                            Text("\(String(code.prefix(3))) \(String(code.suffix(3)))")
                                .font(.system(size: 40, weight: .bold, design: .monospaced))
                        }
                    }
                    if let error { Text(error).foregroundStyle(.red).font(.footnote) }
                    Button(running ? "Parar" : "Iniciar") { running ? stop() : start() }
                        .buttonStyle(.borderedProminent)
                }

                Section(header: Text("Datos para el PC"),
                        footer: Text("Por USB: conecta el cable y abre OpenCam en el ordenador (macOS o Linux con usbmuxd); se conecta solo.")) {
                    if addresses.isEmpty {
                        Text("Sin Wi-Fi. Conéctate a la misma red que el PC o usa USB.")
                    }
                    ForEach(addresses, id: \.ip) { a in
                        HStack {
                            Text("\(a.ip):\(String(OpcmServer.port))").font(.system(.title3, design: .monospaced))
                                .textSelection(.enabled)
                            Spacer()
                            Text(a.kind).foregroundStyle(.secondary)
                        }
                    }
                    Text("Nombre: \(UIDevice.current.name)").foregroundStyle(.secondary)
                }

                Section(header: Text("Ajustes de vídeo"),
                        footer: Text("Se aplican al iniciar. Si el PC pide otra calidad manda la del PC; marca allí «Usar la calidad elegida en el móvil» para usar estos.")) {
                    Picker("Resolución", selection: $settings.width) {
                        ForEach(resolutions, id: \.self) { w in Text("\(w)x\(w * 9 / 16)").tag(w) }
                    }
                    Picker("Fotogramas/s", selection: $settings.fps) {
                        ForEach(StreamSettings.fpsOptions, id: \.self) { f in Text("\(f) fps").tag(f) }
                    }
                    Picker("Calidad", selection: $settings.bitrateKbps) {
                        ForEach(StreamSettings.bitrateOptions, id: \.kbps) { o in Text(o.label).tag(o.kbps) }
                    }
                }
                .disabled(running)
            }
            .navigationTitle("OpenCam")
        }
        .navigationViewStyle(.stack)
        .onAppear {
            addresses = NetInfo.addresses()
            if !resolutions.contains(settings.width) { settings.width = resolutions.last ?? 1920 }
            streamer.onConnectionChange = { value in DispatchQueue.main.async { connected = value } }
        }
        .onChange(of: settings) { s in s.save() }
    }

    private var statusText: String {
        if !running { return "Parado" }
        return connected ? "Conectado al ordenador" : "Esperando al ordenador en el puerto \(OpcmServer.port)"
    }

    private var statusColor: Color { !running ? .gray : connected ? .green : .orange }

    private func start() {
        addresses = NetInfo.addresses()
        settings.save()
        streamer.settings = settings
        do {
            try streamer.start()
            code = streamer.pairingCode
            streamer.onPairingCodeChanged = { c in DispatchQueue.main.async { code = c } }
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
        connected = false
    }
}
