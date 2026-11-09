# OpenCam desktop

Cliente de escritorio: se conecta al móvil por el protocolo OPCM (ver `docs/PROTOCOL.md`), decodifica H.264 y lo entrega a una cámara virtual.

## Estado

| Sistema | Cámara virtual | Estado |
|---|---|---|
| Linux | `v4l2loopback` (`/dev/videoN`) | Implementado. Compila; no probado con el módulo cargado en este entorno. |
| Windows | Filtro DirectShow (`desktop/windows-vcam`) | Implementado: YUY2 a 30 fps en 1920x1080, 1280x720 y 640x480 (`IAMStreamConfig`). Compila para Windows en CI; sin probar en ejecución. |
| macOS | Ninguna | Sin cámara virtual. Salida Y4M por stdout o archivo (`--sink y4m`, por defecto). El binario se compila en CI. Ver la sección macOS. |

## Compilar y probar

```bash
cd desktop
cargo build
cargo test
```

## Windows: cámara virtual DirectShow

Dos piezas:

- El cliente (`desktop/`) escribe cada frame YUYV en la memoria compartida `Local\OpenCamVirtualCamera`, con el tamaño de `--width` y `--height` (por defecto 1920x1080). El formato está en `core/src/shared_frame.rs`.
- El filtro (`desktop/windows-vcam/`, DLL `opencam_vcam.dll`) es una fuente de vídeo DirectShow que lee esa memoria y la entrega a las apps. Si la memoria no existe (el cliente no está en marcha), entrega negro; si el cliente se detiene, repite el último frame. Reabre la memoria cada segundo, así que el cliente puede arrancar o reiniciarse después de la cámara.

El filtro es un crate independiente: `cargo build` en `desktop/` no lo compila.

### Instalador (lo recomendado)

El workflow de release genera `opencam-setup-windows-x86_64.exe` con NSIS (script en `desktop/windows-installer/`). Copia el cliente y la DLL en `C:\Program Files\OpenCam`, registra la cámara virtual y deja accesos directos; al desinstalar, la desregistra.

Para compilarlo a mano, con las dos piezas dentro de `payload\`:

```powershell
cd desktop\windows-installer
makensis /DVERSION=0.1.2 opencam.nsi
```

### Compilar

Con Rust (toolchain MSVC) en Windows:

```powershell
cd desktop\windows-vcam
cargo build --release
```

La DLL queda en `desktop\windows-vcam\target\release\opencam_vcam.dll`.

### Registrar y quitar (como administrador)

```powershell
regsvr32 "C:\ruta\a\opencam_vcam.dll"
regsvr32 /u "C:\ruta\a\opencam_vcam.dll"
```

Las apps de 32 bits necesitan una DLL de 32 bits registrada con `C:\Windows\SysWOW64\regsvr32.exe`. El registro escribe en `HKCR\CLSID` y en la categoría de fuentes de vídeo (`{860BB310-5D01-11D0-BD3B-00A0C911CE86}`).

Después, arranca el cliente y elige **OpenCam Virtual Camera** en la app que use la cámara:

```powershell
cargo run --release
```

En Windows `--device` se ignora.

### Limitaciones

- Formatos YUY2 a 30 fps en 1080p, 720p y 480p (`IAMStreamConfig`); si la memoria compartida trae otra resolución, el filtro escala con letterbox. No hay `FilterData`, y `SetFormat` no reconecta un pin ya conectado.
- No es Media Foundation Virtual Camera; las apps que sólo listan cámaras con Media Foundation no la verán.
- CI compila la DLL y el instalador, pero nadie la ha registrado ni ejecutado en Windows: el código no se ha probado.

## macOS

- Sin cámara virtual. El binario arm64 se compila en CI (job `macos-desktop`).
- Salida Y4M (YUV4MPEG2 4:2:2) a archivo o stdout, conectada a la CLI y por defecto en macOS. Por ejemplo: `cargo run --release -- 127.0.0.1:4747 --output - | ffplay -`.
- Pendiente: una cámara virtual para macOS.

## Linux: preparar la cámara virtual

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
```

## Conectar con el móvil (USB o Wi-Fi)

```bash
cargo run -- --device /dev/video10            # auto: USB (adb) y luego Wi-Fi (mDNS)
cargo run -- usb --device /dev/video10        # solo Android por USB
cargo run -- 192.168.1.20:4747 --device /dev/video10
```

El argumento es cómo llegar al móvil: `auto` (por defecto), `usb`, `wifi`, `iphone` o `host:puerto`. Con `auto` prueba Android por USB (busca `adb` en el PATH, `ANDROID_HOME`, el SDK de Android o junto al ejecutable, y crea el `adb forward` solo), luego iPhone por USB y por último el primer móvil anunciado por mDNS; en cada reconexión vuelve a buscar; `--device` el v4l2loopback. Opciones: `--sink v4l2|windows|y4m` (y4m por defecto en macOS, con `--output archivo` o `-` para stdout), `--list` y `--discover` (mDNS), `--udid`/`--serial` (elige el móvil USB), `--adb <ruta>`, `--width/--height/--fps/--bitrate`, `--no-reconnect`; ver `--help`. El escritorio envía STATS cada segundo y solo decodifica H.264.

### iPhone por USB (macOS/Linux)

Requiere usbmuxd (en macOS viene con el sistema) y la app iOS en primer plano. No necesita `adb`:

```bash
cargo run -- iphone --device /dev/video10
```
