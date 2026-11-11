# OpenCam para iPhone (Fase 2)

App Swift que convierte el iPhone en cámara para el escritorio. Habla el protocolo OPCM v1 de
[`docs/PROTOCOL.md`](../docs/PROTOCOL.md) en el puerto 4747.

## Compilar

```sh
brew install xcodegen
cd ios && xcodegen generate   # crea OpenCam.xcodeproj desde project.yml
open OpenCam.xcodeproj        # elige tu Team en Signing y ejecuta en un iPhone real
```

La cámara no funciona en el simulador. iOS 15+, solo iPhone.

## Archivos

- `project.yml`: proyecto XcodeGen (target iOS, Swift 5, Info.plist propio).
- `OpenCam/OPCM.swift`: cabecera de 24 bytes big-endian y tipos de mensaje (incluye STATS 0x22).
- `OpenCam/OpcmServer.swift`: servidor TCP (Network.framework) con anuncio Bonjour `_opencam._tcp`
  (TXT `platform=ios`, `name`), HELLO/CAPS, CONFIG, STATS, PING/PONG y ERROR `unsupported_version`.
  Un cliente activo; una conexión nueva solo lo sustituye tras autenticarse.
- `OpenCam/Secure.swift`: emparejamiento por código y cifrado AES-GCM (CryptoKit); claves de los PCs
  emparejados en el llavero (solo este dispositivo). Ver «Seguridad» en `docs/PROTOCOL.md`.
- `OpenCam/H264Encoder.swift`: VideoToolbox H.264 en Annex-B; soporta CMBlockBuffer no contiguo y
  cambio de bitrate en caliente.
- `OpenCam/CameraStreamer.swift`: captura AVFoundation. La sesión se configura una sola vez (Parar y
  Iniciar funcionan). CONFIG elige el preset (3840, 1920 o 1280 de ancho), los fps y el bitrate; el
  encoder usa el tamaño real del primer frame. Un cliente nuevo o `KEYFRAME_REQUEST` fuerzan keyframe;
  todo el estado se toca solo en la cola de captura. STATS `{"target_bitrate_kbps":N}` ajusta
  `AverageBitRate`.
- `OpenCam/OpenCamApp.swift`: interfaz en pestañas (Inicio, Conexión, Ajustes): vista previa de lo
  que recibe el PC, dirección del servidor con botón de copiar, código de emparejamiento, ajustes
  rápidos de resolución, fps y calidad. Mantiene la pantalla encendida (`isIdleTimerDisabled`)
  mientras se emite.
- `OpenCam/Theme.swift`: colores, tarjetas, logotipo y la vista previa (`AVCaptureVideoPreviewLayer`).
- `OpenCam/Info.plist`: permisos de cámara y red local, `NSBonjourServices`.

## Conexión por USB

El escritorio macOS/Linux usa usbmuxd: `opencam-desktop iphone`. Abre un túnel al puerto 4747 del
iPhone sin necesidad de Wi-Fi.

## Estado de verificación

- **No compilado ni probado.** El entorno de desarrollo no tiene Xcode ni Swift; el código y
  `project.yml` solo se han revisado a mano. Es probable que haya que corregir algún detalle de API al
  compilar (p. ej. la firma de `NWListener.Service` con el registro TXT).
- El escritorio sí se compila (`cargo build`); el módulo `usbmuxd` se compila solo en Unix.
- Pendiente: compilar y probar en un iPhone real con `opencam-desktop iphone`, y comprobar
  Parar → Iniciar, el cambio de resolución por CONFIG y el descubrimiento Bonjour.
- Limitaciones: la app solo emite en primer plano; no hay audio.
