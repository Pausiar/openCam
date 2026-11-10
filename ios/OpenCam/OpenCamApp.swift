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

private enum Tab: Hashable {
    case home, connection, settings
}

struct ContentView: View {
    let streamer: CameraStreamer
    @State private var tab = Tab.home
    @State private var running = false
    @State private var connected = false
    @State private var error: String?
    @State private var code: String?
    @State private var copied = false
    @State private var settings = StreamSettings.load()
    @State private var addresses: [NetAddress] = []
    private let resolutions = CameraStreamer.supportedResolutions().map { $0[0] }

    var body: some View {
        TabView(selection: $tab) {
            home
                .tabItem { Label("Inicio", systemImage: "house.fill") }
                .tag(Tab.home)
            connection
                .tabItem { Label("Conexión", systemImage: "link") }
                .tag(Tab.connection)
            settingsPage
                .tabItem { Label("Ajustes", systemImage: "gearshape") }
                .tag(Tab.settings)
        }
        .accentColor(Theme.accent)
        .onAppear {
            addresses = NetInfo.addresses()
            if !resolutions.contains(settings.width) { settings.width = resolutions.last ?? 1920 }
            streamer.onConnectionChange = { value in DispatchQueue.main.async { connected = value } }
        }
        .onChange(of: settings) { s in s.save() }
    }

    // MARK: - Inicio

    private var home: some View {
        ScrollView {
            VStack(spacing: 16) {
                header
                StatusPill(color: running ? Theme.green : Color.gray.opacity(0.6), text: statusText)
                previewCard
                serverCard
                quickSettings
                Button { tab = .settings } label: {
                    HStack {
                        Image(systemName: "slider.horizontal.3").foregroundColor(Theme.accent)
                        Text("Ajustes avanzados").font(.headline).foregroundColor(Theme.text)
                        Spacer()
                        Image(systemName: "chevron.right").foregroundColor(Theme.muted)
                    }
                    .card()
                }
                .buttonStyle(.plain)
            }
            .padding(16)
        }
        .background(Theme.background.ignoresSafeArea())
    }

    private var header: some View {
        HStack(alignment: .top) {
            Button { tab = .settings } label: { Image(systemName: "gearshape").font(.title3) }
                .accessibilityLabel("Ajustes")
            Spacer()
            HStack(spacing: 12) {
                LogoView(size: 56)
                VStack(alignment: .leading, spacing: 2) {
                    Text("OpenCam").font(.system(size: 30, weight: .bold)).foregroundColor(Theme.text)
                    Text("Convierte tu móvil en una cámara para PC")
                        .font(.footnote).foregroundColor(Theme.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer()
            Button { tab = .connection } label: { Image(systemName: "questionmark.circle").font(.title3) }
                .accessibilityLabel("Cómo conectar")
        }
        .foregroundColor(Theme.text)
    }

    private var previewCard: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 22, style: .continuous).fill(Theme.preview)
            if connected {
                CameraPreview(session: streamer.captureSession)
                    .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
            } else {
                VStack(spacing: 8) {
                    Image(systemName: running ? "antenna.radiowaves.left.and.right" : "video")
                        .font(.system(size: 34)).foregroundColor(.white.opacity(0.75))
                    Text(running ? "Esperando al ordenador…" : "Pulsa «Iniciar servidor» y abre OpenCam en el PC")
                        .font(.subheadline).foregroundColor(.white.opacity(0.85))
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 24)
                }
            }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .overlay(alignment: .topLeading) {
            Chip(icon: "camera", text: "Cámara trasera").padding(12)
        }
        .overlay(alignment: .bottomLeading) {
            Chip(text: "\(settings.width) × \(settings.height) · \(settings.fps) FPS").padding(12)
        }
    }

    private var serverCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Dirección del servidor").font(.headline).foregroundColor(Theme.text)
            HStack {
                Text(primaryAddress)
                    .font(.system(.body, design: .monospaced))
                    .foregroundColor(Theme.text)
                    .textSelection(.enabled)
                Spacer()
                Button {
                    UIPasteboard.general.string = primaryAddress
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { copied = false }
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc").foregroundColor(Theme.muted)
                }
                .disabled(addresses.isEmpty)
                .accessibilityLabel("Copiar la dirección")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(RoundedRectangle(cornerRadius: 12).fill(Theme.background))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border, lineWidth: 1))

            // Código de emparejamiento: siempre a la vista (el PC lo pide la primera vez).
            if running, let code = code {
                Text("Código de emparejamiento (escríbelo en el PC la primera vez)")
                    .font(.footnote).foregroundColor(Theme.muted)
                Text("\(String(code.prefix(3))) \(String(code.suffix(3)))")
                    .font(.system(size: 36, weight: .bold, design: .monospaced))
                    .foregroundColor(Theme.accentDark)
                    .textSelection(.enabled)
            } else {
                Text(running ? "Preparando el código de emparejamiento…"
                             : "El código de emparejamiento aparece aquí al iniciar el servidor.")
                    .font(.footnote).foregroundColor(Theme.muted)
            }

            if let error { Text(error).font(.footnote).foregroundColor(.red) }

            Button { running ? stop() : start() } label: {
                Label(running ? "Detener servidor" : "Iniciar servidor",
                      systemImage: running ? "stop.fill" : "play.fill")
                    .font(.headline)
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity, minHeight: 50)
                    .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.accent))
            }
            .buttonStyle(.plain)
        }
        .card()
    }

    private var quickSettings: some View {
        HStack(spacing: 10) {
            tile(icon: "display", title: "Resolución",
                 value: "\(settings.width) × \(settings.height)") {
                Picker("Resolución", selection: $settings.width) {
                    ForEach(resolutions, id: \.self) { w in Text("\(w) × \(w * 9 / 16)").tag(w) }
                }
            }
            tile(icon: "square.stack.3d.up", title: "FPS", value: "\(settings.fps)") {
                Picker("FPS", selection: $settings.fps) {
                    ForEach(StreamSettings.fpsOptions, id: \.self) { f in Text("\(f)").tag(f) }
                }
            }
            tile(icon: "dial.medium", title: "Calidad", value: qualityLabel) {
                Picker("Calidad", selection: $settings.bitrateKbps) {
                    ForEach(StreamSettings.bitrateOptions, id: \.kbps) { o in Text(o.label).tag(o.kbps) }
                }
            }
        }
        .disabled(running)
        .opacity(running ? 0.6 : 1)
    }

    private func tile<P: View>(icon: String, title: String, value: String,
                               @ViewBuilder picker: () -> P) -> some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.title2).foregroundColor(Theme.accent)
            Text(title).font(.subheadline.weight(.semibold)).foregroundColor(Theme.text)
            Menu { picker() } label: {
                HStack(spacing: 4) {
                    Text(value).lineLimit(1).minimumScaleFactor(0.7)
                    Image(systemName: "chevron.down").font(.caption2)
                }
                .font(.footnote)
                .foregroundColor(Theme.text)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity)
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border, lineWidth: 1))
            }
        }
        .card(padding: 12)
    }

    // MARK: - Conexión

    private var connection: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                pageTitle("Conexión", "Cómo encuentra el PC a este iPhone")
                VStack(alignment: .leading, spacing: 8) {
                    Label("Por Wi-Fi", systemImage: "wifi").font(.headline).foregroundColor(Theme.text)
                    if addresses.isEmpty {
                        Text("Sin Wi-Fi. Conéctate a la misma red que el PC o usa USB.")
                            .foregroundColor(Theme.muted)
                    }
                    ForEach(addresses, id: \.ip) { a in
                        HStack {
                            Text("\(a.ip):\(String(OpcmServer.port))")
                                .font(.system(.title3, design: .monospaced))
                                .foregroundColor(Theme.text)
                                .textSelection(.enabled)
                            Spacer()
                            Text(a.kind).foregroundColor(Theme.muted)
                        }
                    }
                    Text("Nombre: \(UIDevice.current.name). Si el PC no lo encuentra solo, escribe la dirección en «IP manual».")
                        .font(.footnote).foregroundColor(Theme.muted)
                }
                .card()
                VStack(alignment: .leading, spacing: 8) {
                    Label("Por cable USB", systemImage: "cable.connector").font(.headline).foregroundColor(Theme.text)
                    Text("Conecta el cable y abre OpenCam en el ordenador (macOS, o Linux con usbmuxd); se conecta solo.")
                        .font(.footnote).foregroundColor(Theme.muted)
                }
                .card()
                VStack(alignment: .leading, spacing: 8) {
                    Label("Emparejamiento", systemImage: "key").font(.headline).foregroundColor(Theme.text)
                    Text("La primera vez el PC pide el código de 6 dígitos que sale en Inicio. Después los dos guardan una clave y la conexión va cifrada.")
                        .font(.footnote).foregroundColor(Theme.muted)
                }
                .card()
            }
            .padding(16)
        }
        .background(Theme.background.ignoresSafeArea())
        .onAppear { addresses = NetInfo.addresses() }
    }

    // MARK: - Ajustes

    private var settingsPage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                pageTitle("Ajustes", "Se aplican al iniciar el servidor")
                VStack(alignment: .leading, spacing: 12) {
                    Text("Vídeo").font(.headline).foregroundColor(Theme.text)
                    settingRow("Resolución") {
                        Picker("Resolución", selection: $settings.width) {
                            ForEach(resolutions, id: \.self) { w in Text("\(w) × \(w * 9 / 16)").tag(w) }
                        }
                    }
                    settingRow("Fotogramas por segundo") {
                        Picker("FPS", selection: $settings.fps) {
                            ForEach(StreamSettings.fpsOptions, id: \.self) { f in Text("\(f) fps").tag(f) }
                        }
                    }
                    settingRow("Calidad") {
                        Picker("Calidad", selection: $settings.bitrateKbps) {
                            ForEach(StreamSettings.bitrateOptions, id: \.kbps) { o in Text(o.label).tag(o.kbps) }
                        }
                    }
                    Text("Si el PC pide otra calidad manda la del PC; marca allí «Usar la calidad elegida en el móvil» para usar esta.")
                        .font(.footnote).foregroundColor(Theme.muted)
                }
                .disabled(running)
                .card()
                VStack(alignment: .leading, spacing: 8) {
                    Label("Acerca de", systemImage: "info.circle").font(.headline).foregroundColor(Theme.text)
                    Text("OpenCam \(appVersion) · código abierto (licencia MIT)")
                        .font(.footnote).foregroundColor(Theme.muted)
                    Link("github.com/Pausiar/openCam", destination: URL(string: "https://github.com/Pausiar/openCam")!)
                        .font(.footnote)
                }
                .card()
            }
            .padding(16)
        }
        .background(Theme.background.ignoresSafeArea())
    }

    private func pageTitle(_ title: String, _ subtitle: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.system(size: 28, weight: .bold)).foregroundColor(Theme.text)
            Text(subtitle).font(.subheadline).foregroundColor(Theme.muted)
        }
    }

    private func settingRow<P: View>(_ title: String, @ViewBuilder picker: () -> P) -> some View {
        HStack {
            Text(title).foregroundColor(Theme.text)
            Spacer()
            picker().pickerStyle(.menu)
        }
    }

    // MARK: - Estado

    private var primaryAddress: String {
        addresses.first.map { "\($0.ip):\(String(OpcmServer.port))" } ?? "Sin Wi-Fi"
    }

    private var qualityLabel: String {
        StreamSettings.bitrateOptions.first { $0.kbps == settings.bitrateKbps }
            .map { $0.label.components(separatedBy: " (").first ?? $0.label } ?? "\(settings.bitrateKbps) kbps"
    }

    private var appVersion: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String).map { "v\($0)" } ?? ""
    }

    private var statusText: String {
        if !running { return "Servidor parado" }
        return connected ? "Conectado al PC" : "Servidor activo"
    }

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
