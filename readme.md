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

- **Resolución, fps y bitrate a tu elección**: 1080p, 720p o 480p, a 30 fps o más si tu móvil lo admite, con el bitrate que quieras (`--width`, `--height`, `--fps`, `--bitrate`).
- **USB o Wi-Fi, sin configurar nada**: el cliente busca el móvil solo. Primero Android por USB (hace él mismo el `adb forward`), luego iPhone por USB (usbmuxd) y luego la Wi-Fi (mDNS).
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
| Windows (cámara virtual suelta) | [`opencam-vcam-windows-x86_64.dll`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-vcam-windows-x86_64.dll) | Para registrarla a mano con `regsvr32`. |
| Linux | [`opencam-desktop-linux-x86_64`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-desktop-linux-x86_64) | Compila y tiene tests. Sin probar con el módulo cargado. |
| macOS (Apple Silicon) | [`opencam-desktop-macos-arm64-…-solo-y4m`](https://github.com/Pausiar/openCam/releases/latest/download/opencam-desktop-macos-arm64-sin-camara-virtual-solo-y4m) | Sin cámara virtual todavía: saca vídeo Y4M. |
| iPhone | Sin descarga | Se compila desde [`ios/`](ios/README.md) con Xcode. |

> [!NOTE]
> OpenCam está en desarrollo. La columna **Estado** dice con sinceridad qué se ha probado y qué solo compila. Si lo pruebas en tu equipo, [cuéntanos cómo te ha ido](https://github.com/Pausiar/openCam/issues).

## Cómo empezar

### 1. Prepara el móvil

**Android**

1. Instala la APK (Android te pedirá permitir instalar apps de fuentes desconocidas).
2. Abre OpenCam, da permiso de cámara y pulsa **Iniciar**.

**iPhone**

1. Compila la app desde [`ios/`](ios/README.md) con XcodeGen y Xcode, en un Mac con tu firma de Apple.
2. Ábrela y pulsa **Iniciar**. Déjala en primer plano: iOS no deja usar la cámara en segundo plano.

### 2. Conecta el ordenador

> [!TIP]
> La búsqueda automática del móvil llega en la versión siguiente a v0.2.1. Con v0.2.1, conecta Android por USB con `adb forward tcp:4747 tcp:4747` y `opencam-desktop 127.0.0.1:4747`.

<details open>
<summary><b>Windows</b></summary>

1. Ejecuta `opencam-setup-windows-x86_64.exe`. Instala el cliente y registra la cámara virtual **OpenCam Virtual Camera**.
2. Arranca `opencam-desktop` sin argumentos. Busca el móvil solo: Android por USB, iPhone por USB y después la Wi-Fi.
   - Para Android por USB activa la **depuración USB** en el móvil y ten instaladas las [platform-tools](https://developer.android.com/tools/releases/platform-tools) (`adb`). Si `adb` no está en el PATH, indícalo con `--adb C:\ruta\adb.exe`.
   - Para forzar un modo: `opencam-desktop usb`, `opencam-desktop wifi`, `opencam-desktop iphone` o `opencam-desktop 192.168.1.50:4747`.
3. En Zoom, Meet, OBS… elige **OpenCam Virtual Camera**.

</details>

<details>
<summary><b>Linux</b></summary>

```bash
# Crea la cámara virtual (una vez por arranque)
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1

# Busca el móvil solo: Android por USB (necesita adb), iPhone por USB (usbmuxd) o Wi-Fi
./opencam-desktop-linux-x86_64 --device /dev/video10

# O fuerza un modo concreto
./opencam-desktop-linux-x86_64 usb --device /dev/video10
./opencam-desktop-linux-x86_64 192.168.1.50:4747 --device /dev/video10
```

Luego elige **OpenCam** como cámara en tu app.

</details>

<details>
<summary><b>macOS</b></summary>

Todavía no hay cámara virtual en macOS. El cliente saca el vídeo en formato Y4M, que puedes ver con `ffplay` o meter en OBS:

```bash
./opencam-desktop-macos-arm64-sin-camara-virtual-solo-y4m | ffplay -
```

El binario no va firmado: la primera vez ábrelo con clic derecho → **Abrir**.

</details>

### 3. Ajusta la calidad

```bash
opencam-desktop --width 1280 --height 720 --fps 60 --bitrate 6000
```

| Opción | Por defecto | Qué hace |
|---|---|---|
| `--width` / `--height` | `1920` × `1080` | Resolución que se pide al móvil. |
| `--fps` | `30` | Fotogramas por segundo. |
| `--bitrate` | `8000` | Bitrate de vídeo en kbps. Bájalo si la Wi-Fi va justa. |
| `--device` | `/dev/video10` | Dispositivo v4l2loopback (Linux). |
| `--sink` | según el sistema | `v4l2`, `windows` o `y4m`. |
| (dirección) | `auto` | `auto`, `usb`, `wifi`, `iphone` o `IP:4747`. |
| `--serial` | | Elige el móvil (serie de Android o UDID de iPhone) si hay varios. |
| `--adb` | | Ruta a `adb` si no está en el PATH. |
| `--discover` | | Igual que `wifi`: busca móviles en la red local. |
| `--list` | | Lista los móviles disponibles (Android e iPhone por USB, y Wi-Fi). |
| `--no-reconnect` | | No reintenta la conexión si se corta. |

La lista completa está en `opencam-desktop --help`.

<!-- CAPTURA PENDIENTE: site/img/captura-ajustes.png (terminal con el cliente conectado mostrando resolución y fps) -->

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
