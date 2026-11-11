<div align="center">

# OpenCam

**Usa tu móvil como webcam del ordenador. Libre, sin anuncios y sin marcas de agua.**

[![Última release](https://img.shields.io/github/v/release/Pausiar/openCam?label=descargar&color=1d5e4b)](https://github.com/Pausiar/openCam/releases/latest)
[![CI](https://github.com/Pausiar/openCam/actions/workflows/ci.yml/badge.svg)](https://github.com/Pausiar/openCam/actions/workflows/ci.yml)
[![Web](https://img.shields.io/badge/web-pausiar.github.io%2FopenCam-1d5e4b)](https://pausiar.github.io/openCam/)

[Web y descargas](https://pausiar.github.io/openCam/) ·
[Releases](https://github.com/Pausiar/openCam/releases/latest) ·
[Cómo empezar](#cómo-empezar) ·
[Compilar](#compilar-desde-el-código)

</div>

OpenCam convierte un móvil Android o un iPhone en la cámara web de tu PC. El móvil graba y comprime el vídeo por hardware, lo manda por **USB o Wi-Fi**, y el cliente de escritorio lo publica como **cámara virtual**. Así cualquier app de vídeo (Zoom, Meet, Teams, OBS, Discord…) la ve como una webcam más.

<p align="center">
  <img src="site/img/como-funciona.svg" alt="El móvil captura y codifica H.264, lo envía por USB o Wi-Fi al ordenador, y el cliente lo publica como cámara virtual" width="720">
</p>

<!--
  CAPTURAS PENDIENTES. Cuando las tengas, guárdalas en site/img/ y descomenta este bloque:

<p align="center">
  <img src="site/img/captura-android.png" alt="App de Android emitiendo" width="240">
  <img src="site/img/captura-escritorio.png" alt="OpenCam como cámara en una videollamada" width="520">
</p>
-->

## Qué ofrece

- **Ventana sencilla en el ordenador** (Windows, Linux y macOS) para conectar el móvil, elegir la calidad y ver una vista previa; y una pantalla en el móvil con la IP y el puerto que hay que poner.
- **Resolución, fps y bitrate a tu elección**: 1080p, 720p o 480p, a 30 fps o más si tu móvil lo admite, con el bitrate que quieras, desde el móvil o desde el PC.
- **USB o Wi-Fi**: por cable con `adb forward` (Android) o usbmuxd (iPhone), o por la red local. En Wi-Fi el ordenador encuentra el móvil solo (mDNS, `--discover`).
- **Vídeo H.264 por hardware**: el móvil apenas se calienta y la latencia es baja.
- **Reconexión automática**: si se corta el cable o la Wi-Fi, el cliente vuelve a conectar sin que reinicies nada.
- **Cámara virtual de verdad** en Windows (DirectShow) y Linux (v4l2loopback).
- **Código abierto**: protocolo documentado ([`docs/PROTOCOL.md`](docs/PROTOCOL.md)) y nada de cuentas ni telemetría.

## Descargas

Todo sale de la [última release](https://github.com/Pausiar/openCam/releases/latest). También puedes descargar desde la [web del proyecto](https://pausiar.github.io/openCam/).

| Plataforma | Archivo | Estado |
|---|---|---|
| Android | [`opencam-android.apk`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-android.apk) | Tests en CI. Sin probar en un móvil real. |
| Windows (instalador) | [`opencam-setup-windows-x86_64.exe`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-setup-windows-x86_64.exe) | Compila en CI. Sin probar en Windows. |
| Windows (cliente suelto) | [`opencam-desktop-windows-x86_64.exe`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-desktop-windows-x86_64.exe) | Igual que el instalador, sin registrar la cámara. |
| Windows (cámara virtual suelta) | [`opencam-vcam-windows-x86_64.dll`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-vcam-windows-x86_64.dll) y [`opencam-vcam-windows-x86.dll`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-vcam-windows-x86.dll) | 64 y 32 bits, para registrarlas a mano con `regsvr32` (ver [desktop/README.md](desktop/README.md)). El instalador ya registra las dos. |
| Linux | [`opencam-desktop-linux-x86_64`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-desktop-linux-x86_64) | Compila y tiene tests. Sin probar con el módulo cargado. |
| macOS (Apple Silicon) | [`opencam-desktop-macos-arm64-…-solo-y4m`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-desktop-macos-arm64-sin-camara-virtual-solo-y4m) | Sin cámara virtual todavía: saca vídeo Y4M. |
| iPhone | Sin descarga | Se compila desde [`ios/`](ios/README.md) con Xcode. |

> [!NOTE]
> OpenCam está en desarrollo. La columna **Estado** dice con sinceridad qué se ha probado y qué solo compila. Si lo pruebas en tu equipo, [cuéntanos cómo te ha ido](https://github.com/Pausiar/openCam/issues).

## Cómo empezar

### 1. Prepara el móvil

1. Instala la APK (Android te pedirá permitir apps de fuentes desconocidas). En iPhone, compila la app desde [`ios/`](ios/README.md) con XcodeGen y Xcode.
2. Abre OpenCam. En **Inicio** verás la **dirección del servidor** (p. ej. `192.168.1.50:4747`, con botón para copiarla); en **Conexión**, todas las direcciones, el nombre del móvil y cómo conectar por USB.
3. Elige resolución, fps y calidad en los ajustes rápidos (y, si quieres, zoom, enfoque o estabilización en **Ajustes**), y pulsa **Iniciar servidor**. Aparece el código de emparejamiento de 6 dígitos. En iPhone, deja la app en primer plano: iOS no deja usar la cámara en segundo plano.

### 2. Conecta el ordenador

Abre **OpenCam** en el ordenador (en Windows, desde el acceso directo del instalador). En **Inicio**:

1. En la tarjeta **Conexión** elige *Auto* (prueba USB y luego Wi-Fi), *Wi-Fi* (el primer móvil que encuentre, o uno de la lista con *Buscar*), *USB* (Android con depuración USB y adb, o iPhone en macOS y Linux) o *IP manual* (la dirección que muestra el móvil).
2. La primera vez, escribe en **Código de emparejamiento** los 6 dígitos que muestra el móvil; después el PC queda emparejado y la conexión va cifrada.
3. Ajusta la resolución de la cámara virtual y los fps en las tarjetas de abajo; en **Ajustes avanzados**, el zoom, la calidad o *Usar la calidad elegida en el móvil*. La salida (cámara virtual, archivo Y4M o solo vista previa) está en **Ajustes**.
4. Pulsa **Conectar**. Verás el estado, los fps, la latencia y una vista previa, y podrás cambiar en vivo la cámara (frontal o trasera), la estabilización y voltear la imagen.
5. En **Dispositivos** puedes buscar los móviles de tu red y elegir uno.
6. En Zoom, Meet, OBS… elige **OpenCam Virtual Camera** (Windows) u **OpenCam** (Linux).

<details>
<summary><b>Linux: crea antes la cámara virtual</b></summary>

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
```

</details>

<details>
<summary><b>macOS</b></summary>

Todavía no hay cámara virtual en macOS: la ventana muestra la vista previa y puede grabar a un archivo Y4M. El binario no va firmado: la primera vez ábrelo con clic derecho → **Abrir**.

</details>

### 3. Desde la terminal (opcional)

Con argumentos, el cliente funciona sin ventana:

```bash
opencam-desktop auto --device /dev/video10                         # busca el móvil (USB y Wi-Fi)
opencam-desktop 192.168.1.50:4747 --width 1280 --height 720 --fps 60 --bitrate 6000
opencam-desktop usb --phone-settings                               # usa la calidad elegida en el móvil
opencam-desktop 192.168.1.50:4747 --sink y4m | ffplay -            # macOS: ver el vídeo
```

| Opción | Por defecto | Qué hace |
|---|---|---|
| `--width` / `--height` | `1920` × `1080` | Resolución de la cámara virtual (y la que se pide al móvil). |
| `--fps` | `30` | Fotogramas por segundo. |
| `--bitrate` | `8000` | Bitrate de vídeo en kbps. Bájalo si la Wi-Fi va justa. |
| `--phone-settings` | | No pide resolución, fps ni bitrate: usa los del móvil. |
| `--device` | `/dev/video10` | Dispositivo v4l2loopback (Linux). |
| `--sink` | según el sistema | `v4l2`, `windows`, `y4m` o `none`. |
| `--list` | | Lista los móviles disponibles. |
| `--no-reconnect` | | No reintenta la conexión si se corta. |

La lista completa (zoom, enfoque, cámara frontal…) está en `opencam-desktop --help`.

## Estado por plataforma

| Plataforma | Qué hay | Verificado |
|---|---|---|
| Android | App Kotlin con Camera2 y MediaCodec. USB (`adb forward`) y Wi-Fi. Anuncio por NSD. | APK firmada y tests unitarios en CI. Sin probar en un móvil real. |
| iPhone | App Swift con AVFoundation y VideoToolbox. USB (usbmuxd) y Wi-Fi con Bonjour. | Compila en CI para el simulador. Sin probar en un iPhone. |
| Linux | Cliente Rust que publica en `/dev/videoN` con v4l2loopback. | Compila y tiene tests. Sin probar con el módulo cargado. |
| Windows | Cliente Rust, filtro DirectShow (`opencam_vcam.dll`) e instalador NSIS. | Compila en CI. Sin ejecutar en Windows. |
| macOS | Cliente Rust con salida Y4M. | Compila en CI. Sin cámara virtual. |

Más detalle en [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`desktop/README.md`](desktop/README.md) e [`ios/README.md`](ios/README.md).

## Compilar desde el código

```bash
cd core && cargo test                 # protocolo, cola, métricas, bitrate, reconexión
cd desktop && cargo build --release   # cliente de escritorio
cd desktop && cargo test
cd android && ./gradlew :app:assembleDebug
cd ios && xcodegen generate           # luego abre OpenCam.xcodeproj en Xcode (macOS)
```

La cámara virtual de Windows es un crate aparte: `cd desktop/windows-vcam && cargo build --release` (en Windows, toolchain MSVC).

Las releases se generan solas al subir un tag `v*` (ver [`.github/workflows/release.yml`](.github/workflows/release.yml)). La web de [`site/`](site/index.html) se publica en GitHub Pages en cada cambio a `main`.

## Documentación

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): arquitectura, fases y estado.
- [`docs/PROTOCOL.md`](docs/PROTOCOL.md): protocolo OPCM v1 entre móvil y escritorio.

## Contribuir

Los reportes de prueba en hardware real son lo que más ayuda ahora mismo. Abre un [issue](https://github.com/Pausiar/openCam/issues) con tu móvil, tu sistema y lo que viste.

## Licencia

El repositorio no incluye todavía un fichero de licencia.
