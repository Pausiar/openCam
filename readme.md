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

### En el móvil

1. Instala la APK (Android) y permite instalar desde fuentes desconocidas. En iPhone, compila la app desde `ios/` con XcodeGen y Xcode (no hay descarga).
2. Abre OpenCam. La pantalla muestra los **datos para el PC**: la IP y el puerto (`192.168.1.20:4747`), el nombre del móvil y cómo conectar por USB.
3. Elige resolución, fotogramas por segundo, calidad y cámara, y pulsa **Iniciar**. Los ajustes se guardan.

### En el ordenador

1. Abre **OpenCam** (sin argumentos se abre la ventana; en Windows, desde el acceso directo que deja el instalador).
2. Elige cómo conectar: **Automática** (USB y luego Wi-Fi), **Wi-Fi** (escribe la IP que muestra el móvil o pulsa *Buscar móviles en la red*), **Android por USB** (con la depuración USB activada y adb instalado) o **iPhone por USB** (macOS y Linux).
3. Elige la resolución de la cámara virtual, los fps y la calidad, o marca **Usar la calidad elegida en el móvil**.
4. Pulsa **Conectar**. La ventana muestra el estado, los fps, la latencia y una vista previa.
5. En la app de vídeo, elige la cámara: **OpenCam Virtual Camera** en Windows o `/dev/video10` en Linux. En macOS aún no hay cámara virtual: la ventana muestra la vista previa y puede grabar a Y4M.

En Linux, carga antes el módulo de la cámara virtual:

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
```

### Línea de órdenes

Con argumentos, `opencam-desktop` funciona como antes, sin ventana:

```bash
opencam-desktop                                  # ventana
opencam-desktop auto --device /dev/video10       # busca el móvil (USB y Wi-Fi)
opencam-desktop 192.168.1.20:4747 --fps 60       # dirección fija
opencam-desktop usb --phone-settings             # usa la calidad elegida en el móvil
```

Las opciones completas están en `opencam-desktop --help`. iOS no captura cámara en segundo plano: deja la app del iPhone en primer plano.

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
