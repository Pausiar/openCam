# OpenCam

OpenCam convierte un móvil Android o un iPhone en la webcam de tu ordenador. El móvil captura y codifica el vídeo por hardware; el escritorio lo recibe y lo publica como cámara virtual, para que cualquier app de vídeo la pueda usar.

El proyecto está en desarrollo. Lo que sigue dice qué está probado de verdad y qué solo compila o está revisado.

## Estado por plataforma

| Plataforma | Qué hay | Verificado |
|---|---|---|
| Android (móvil) | App Kotlin con Camera2 y MediaCodec. Transporte USB (`adb forward`) y Wi-Fi. Anuncia el servicio por NSD. | APK de debug y tests unitarios en CI. Sin probar en un móvil real. |
| iPhone (móvil) | App Swift con AVFoundation y VideoToolbox. Transporte USB (usbmuxd) y Wi-Fi con Bonjour. | Proyecto XcodeGen revisado. No compilado: el entorno de desarrollo no tiene Xcode. |
| Linux (escritorio) | Cliente Rust que publica en `/dev/videoN` con v4l2loopback, e instalador del módulo (`desktop/linux/install.sh`). | Compila y tiene tests. Sin probar con el módulo cargado. |
| Windows (escritorio) | Cliente Rust, filtro DirectShow (`opencam_vcam.dll`) e instalador NSIS. | Compila en CI. Sin ejecutar ni registrar en Windows. |
| macOS (escritorio) | Cliente Rust y `OpenCam.app` con una extensión de cámara CoreMediaIO (`macos/`). | Compila en CI (sin firmar). Sin probar en un Mac; hay que compilarla con una cuenta de Apple Developer. |

Detalles de cada pieza y de lo que falta: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`desktop/README.md`](desktop/README.md), [`macos/README.md`](macos/README.md) e [`ios/README.md`](ios/README.md).

## Descargas

Las apps compiladas se publican en las [releases de GitHub](https://github.com/Pausiar/openCam/releases/latest). La página de descargas está en [`site/`](site/index.html) y se publica con GitHub Pages.

## Uso rápido

### Android

1. Instala la APK y permite instalar desde fuentes desconocidas.
2. Abre OpenCam y pulsa **Iniciar**.
3. Por USB, ejecuta `adb forward tcp:4747 tcp:4747` y conecta el escritorio a `127.0.0.1:4747`. Por Wi-Fi, usa la IP del móvil en el puerto 4747.

### iPhone (Linux y macOS)

1. Compila la app desde `ios/` con XcodeGen y Xcode, en un Mac con tu firma de Apple. No hay descarga.
2. Abre la app en primer plano y pulsa **Iniciar**. iOS no captura cámara en segundo plano.
3. Conecta por USB (en Linux con usbmuxd instalado): `opencam-desktop iphone`.

### Linux

Con la release (`opencam-linux-x86_64.tar.gz`):

```bash
tar xzf opencam-linux-x86_64.tar.gz && cd opencam-linux-x86_64
sudo ./install.sh ./opencam-desktop   # instala v4l2loopback y crea la cámara "OpenCam"
opencam-desktop 127.0.0.1:4747
```

Desde el código: `sudo desktop/linux/install.sh` y luego `cd desktop && cargo run --release -- 127.0.0.1:4747`. Las opciones completas están en `opencam-desktop --help`.

### macOS

1. Compila `OpenCam.app` desde `macos/` con tu cuenta de Apple Developer y cópiala a Aplicaciones ([`macos/README.md`](macos/README.md)).
2. Ábrela, pulsa **Instalar cámara** y acepta el permiso en Ajustes del Sistema.
3. Arranca el cliente: `opencam-desktop 127.0.0.1:4747` (o `iphone`). En la app de vídeo, elige **OpenCam**.

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
cd macos && xcodegen generate      # cámara virtual de macOS: abre OpenCamMac.xcodeproj
```

El filtro de Windows es un crate aparte: `cd desktop/windows-vcam && cargo build --release` en Windows.

## Documentación

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): arquitectura, fases y estado.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md): protocolo OPCM v1 entre móvil y escritorio.

## Licencia

El repositorio no incluye todavía un fichero de licencia.
