# OpenCam desktop

Cliente de escritorio: se conecta al móvil por el protocolo OPCM (ver `docs/PROTOCOL.md`), decodifica H.264 y lo entrega a una cámara virtual.

## Estado

| Sistema | Cámara virtual | Estado |
|---|---|---|
| Linux | `v4l2loopback` (`/dev/videoN`) | Implementado, con instalador (`linux/install.sh`), detección automática del dispositivo y fps anunciados. Compila y tiene tests; no probado con el módulo cargado. |
| Windows | Filtro DirectShow (`desktop/windows-vcam`) | Implementado: YUY2 a 30 fps en 1920x1080, 1280x720 y 640x480 (`IAMStreamConfig`). Compila para Windows en CI; sin probar en ejecución. |
| macOS | Extensión de cámara CoreMediaIO (`macos/`, `OpenCam.app`) | Implementado: 1920x1080 YUYV a 30 fps. El cliente y la app compilan en CI; sin probar en un Mac. Necesita firma de Apple Developer. Ver la sección macOS. |

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
cargo run --release -- 127.0.0.1:4747
```

En Windows `--device` se ignora.

### Limitaciones

- Formatos YUY2 a 30 fps en 1080p, 720p y 480p (`IAMStreamConfig`); si la memoria compartida trae otra resolución, el filtro escala con letterbox. No hay `FilterData`, y `SetFormat` no reconecta un pin ya conectado.
- No es Media Foundation Virtual Camera; las apps que sólo listan cámaras con Media Foundation no la verán.
- CI compila la DLL y el instalador, pero nadie la ha registrado ni ejecutado en Windows: el código no se ha probado.

## macOS: cámara virtual

- La cámara es una extensión de CoreMediaIO dentro de `OpenCam.app`. Cómo compilarla, firmarla e instalarla: [`macos/README.md`](../macos/README.md).
- El sink `macos` (por defecto en macOS) lanza `OpenCam.app/Contents/MacOS/OpenCam --feed` y le pasa los frames por stdin. Busca la app en `/Applications` y `~/Applications`; otra ruta con `OPENCAM_FEEDER`.
- Formato fijo 1920x1080 a 30 fps; otra resolución del móvil se escala con letterbox.
- Sigue disponible la salida Y4M: `cargo run --release -- 127.0.0.1:4747 --sink y4m --output - | ffplay -`.

## Linux: preparar la cámara virtual

Con el instalador (Debian/Ubuntu, Fedora con RPM Fusion, Arch, openSUSE):

```bash
sudo desktop/linux/install.sh                       # instala v4l2loopback y crea /dev/video10 "OpenCam"
sudo desktop/linux/install.sh ./opencam-desktop     # además copia el cliente a /usr/local/bin
sudo desktop/linux/install.sh --uninstall
```

La release trae `opencam-linux-x86_64.tar.gz` con el cliente y el instalador:
`tar xzf opencam-linux-x86_64.tar.gz && cd opencam-linux-x86_64 && sudo ./install.sh ./opencam-desktop`.

El instalador deja la configuración en `/etc/modprobe.d/opencam.conf` y `/etc/modules-load.d/opencam.conf`, así que la cámara vuelve en cada arranque. Con Secure Boot hay que firmar el módulo DKMS (la distribución lo pide al instalarlo).

A mano, sin instalador:

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
```

`--device auto` (por defecto) usa el dispositivo llamado OpenCam, o el primer v4l2loopback que encuentre. El cliente anuncia los fps de `--fps` a las apps. Con `exclusive_caps=1`, Chrome y otras apps solo ven la cámara mientras el cliente está emitiendo.

## Uso con el móvil por USB

```bash
adb forward tcp:4747 tcp:4747
cargo run -- 127.0.0.1:4747
```

El argumento es la dirección del móvil; `--device` el v4l2loopback (`auto` por defecto). Opciones: `--sink v4l2|windows|macos|y4m` (por defecto el del sistema; y4m con `--output archivo` o `-` para stdout), `--list` y `--discover` (mDNS), `--udid` (iPhone USB), `--width/--height/--fps/--bitrate`, `--no-reconnect`; ver `--help`. El escritorio envía STATS cada segundo y solo decodifica H.264.

### iPhone por USB (macOS/Linux)

Requiere usbmuxd (en macOS viene con el sistema) y la app iOS en primer plano. No necesita `adb`:

```bash
cargo run -- iphone
```
