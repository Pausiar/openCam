# OpenCam

OpenCam convierte un móvil Android o un iPhone en la webcam de tu ordenador. El móvil captura y codifica el vídeo por hardware; el escritorio lo recibe y lo publica como cámara virtual, para que cualquier app de vídeo la pueda usar.

El proyecto está en desarrollo. Lo que sigue dice qué está probado de verdad y qué solo compila o está revisado.

## Estado por plataforma

| Plataforma | Qué hay | Verificado |
|---|---|---|
| Android (móvil) | App Kotlin con Camera2 y MediaCodec. Transporte USB (`adb forward`) y Wi-Fi. Anuncia el servicio por NSD. | APK de debug y tests unitarios en CI. Sin probar en un móvil real. |
| iPhone (móvil) | App Swift con AVFoundation y VideoToolbox. Transporte USB (usbmuxd) y Wi-Fi con Bonjour. | Proyecto XcodeGen revisado. No compilado: el entorno de desarrollo no tiene Xcode. |
| Linux (escritorio) | Cliente Rust que publica en `/dev/videoN` con v4l2loopback. | Compila y tiene tests. Sin probar con el módulo cargado. |
| Windows (escritorio) | Cliente Rust, filtro DirectShow (`opencam_vcam.dll`) e instalador NSIS. | Compila en CI. Sin ejecutar ni registrar en Windows. |
| macOS (escritorio) | Cliente Rust sin cámara virtual. | Compila en CI. Sale a Y4M por archivo o stdout (`--sink y4m`, por defecto en macOS); sin cámara virtual real. |

Detalles de cada pieza y de lo que falta: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`desktop/README.md`](desktop/README.md) e [`ios/README.md`](ios/README.md).

## Descargas

Las apps compiladas se publican en las [releases de GitHub](https://github.com/Pausiar/openCam/releases/latest). La página de descargas está en [`site/`](site/index.html) y se publica con GitHub Pages.

## Uso rápido

### Android

1. Instala la APK y permite instalar desde fuentes desconocidas.
2. Abre OpenCam y pulsa **Iniciar**.
3. Por USB, ejecuta `adb forward tcp:4747 tcp:4747` y conecta el escritorio a `127.0.0.1:4747`. Por Wi-Fi, usa la IP del móvil en el puerto 4747.

### iPhone (solo Linux, por ahora)

1. Compila la app desde `ios/` con XcodeGen y Xcode, en un Mac con tu firma de Apple. No hay descarga.
2. Abre la app en primer plano y pulsa **Iniciar**. iOS no captura cámara en segundo plano.
3. Conecta por USB con usbmuxd instalado: `opencam-desktop iphone --device /dev/video10`.

### Linux

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
cd desktop
cargo run --release -- --addr 127.0.0.1:4747 --device /dev/video10
```

Las opciones completas están en `opencam-desktop --help`.

### Windows

1. Instala `opencam-setup-windows-x86_64.exe`. Copia el cliente y la cámara virtual, y la registra.
2. Arranca el cliente con la dirección del móvil (`127.0.0.1:4747` con `adb forward`).
3. En la app que use la cámara, elige **OpenCam Virtual Camera**.

## Compilar y probar

```bash
cd core && cargo test              # protocolo, cola, métricas, bitrate, reconexión
cd desktop && cargo build --release
cd desktop && cargo test
cd android && ./gradlew :app:assembleDebug
cd ios && xcodegen generate        # luego abre OpenCam.xcodeproj en Xcode (requiere macOS)
```

El filtro de Windows es un crate aparte: `cd desktop/windows-vcam && cargo build --release` en Windows.

## Documentación

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): arquitectura, fases y estado.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md): protocolo OPCM v1 entre móvil y escritorio.

## Licencia

El repositorio no incluye todavía un fichero de licencia.
