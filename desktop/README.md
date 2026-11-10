# OpenCam desktop

Cliente de escritorio: se conecta al móvil por el protocolo OPCM (ver `docs/PROTOCOL.md`), decodifica H.264 y lo entrega a una cámara virtual.

## Interfaz gráfica

Sin argumentos, `opencam-desktop` abre una ventana (egui) en Windows, Linux y macOS:

- **Inicio**: vista previa (con resolución, fps y latencia, y un botón para cambiar de cámara), tarjeta de **Conexión** (Auto, Wi-Fi con búsqueda mDNS, USB para Android o iPhone, o IP manual) con el código de emparejamiento y el botón **Conectar/Desconectar**, ajustes rápidos (resolución de la cámara virtual 1920x1080, 1280x720 o 640x480, fps, cámara, voltear imagen, estabilización) y **Ajustes avanzados** plegables (zoom, bitrate, *Usar la calidad elegida en el móvil*, reconexión).
- **Dispositivos**: móviles encontrados en la red e instrucciones para el cable.
- **Ajustes**: salida (cámara virtual v4l2loopback o DirectShow, archivo Y4M o solo vista previa, lo normal en macOS) y el registro de la conexión.
- **Acerca de**: versión y enlaces.

La interfaz usa las fuentes de `assets/fonts/` (Inter e iconos, incrustadas en el binario). La versión que muestra sale de `OPENCAM_VERSION` al compilar (la release pone el tag) o de `Cargo.toml`.

Los ajustes se guardan en `%APPDATA%\OpenCam\settings.txt` (Windows), `~/Library/Application Support/OpenCam/settings.txt` (macOS) o `~/.config/opencam/settings.txt` (Linux).

Con cualquier argumento funciona como CLI (`--gui` fuerza la ventana). En Windows el ejecutable es de ventana: lanzado desde una consola con argumentos, escribe en esa consola, pero la consola no espera a que termine.

En Linux la ventana necesita en tiempo de ejecución OpenGL y `libxkbcommon-x11` (o Wayland), que trae cualquier escritorio.

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

## Emparejar con el móvil

La conexión va cifrada y el móvil solo acepta PCs emparejados. La primera vez, abre la app en
el móvil, mira el código de 6 dígitos y pásalo con `--code`:

```bash
opencam-desktop 192.168.1.20:4747 --code 123456
```

El PC guarda la clave en `~/.config/opencam/pairings` (Linux), `~/Library/Application
Support/opencam/pairings` (macOS) o `%APPDATA%\opencam\pairings` (Windows), con permisos solo
para tu usuario. Las siguientes veces no hace falta el código. Si el código es incorrecto, el
programa se detiene en vez de reintentar (cada intento fallido cuenta y a los 5 el móvil
genera otro código). Detalles en `docs/PROTOCOL.md`, sección «Seguridad».

## Ajustes avanzados de vídeo

Al arrancar:

```bash
opencam-desktop 127.0.0.1:4747 --bitrate 6000 --camera frontal --zoom 1.5 \
    --focus auto --stabilization on --mirror
```

`--camera`, `--zoom`, `--focus` y `--stabilization` solo se envían si los indicas; si no, el móvil mantiene lo elegido en su pantalla. `--mirror` voltea la imagen en el PC. `--codec` admite de momento solo `h264`, porque el escritorio aún no decodifica H.265.

En vivo, escribe en la consola y pulsa Intro (se desactiva con `--no-controls`):

```text
zoom 2            enfoque auto | enfoque 0.4     estabilizacion on|off
camara frontal    espejo on|off                  bitrate 6000           ayuda
```

Los cambios de zoom, enfoque, estabilización y bitrate se aplican sin cortar el vídeo; cambiar de cámara reinicia la captura en el móvil (un par de frames). Lo cambiado se conserva si la conexión se reconecta.

## Windows: cámara virtual DirectShow

Dos piezas:

- El cliente (`desktop/`) escribe cada frame YUYV en la memoria compartida `Local\OpenCamVirtualCamera`, con el tamaño de `--width` y `--height` (por defecto 1920x1080). El formato está en `core/src/shared_frame.rs`.
- El filtro (`desktop/windows-vcam/`, DLL `opencam_vcam.dll`) es una fuente de vídeo DirectShow que lee esa memoria y la entrega a las apps. Si la memoria no existe (el cliente no está en marcha), entrega negro; si el cliente se detiene, repite el último frame. Reabre la memoria cada segundo, así que el cliente puede arrancar o reiniciarse después de la cámara.

El filtro es un crate independiente: `cargo build` en `desktop/` no lo compila.

### Instalador (lo recomendado)

El workflow de release genera `opencam-setup-windows-x86_64.exe` con NSIS (script en `desktop/windows-installer/`). Copia el cliente y las dos DLL (`opencam_vcam.dll` de 64 bits y `opencam_vcam_x86.dll` de 32 bits) en `C:\Program Files\OpenCam`, registra las dos y deja accesos directos; al desinstalar, las desregistra. Hacen falta las dos: cada app sólo carga la DLL de su arquitectura, así que una app de 32 bits no ve una cámara registrada sólo en 64 bits.

Para compilarlo a mano, con las dos piezas dentro de `payload\`:

```powershell
cd desktop\windows-installer
makensis /DVERSION=0.1.2 opencam.nsi
```

### Compilar

Con Rust (toolchain MSVC) en Windows:

```powershell
cd desktop\windows-vcam
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc
```

Las DLL quedan en `target\x86_64-pc-windows-msvc\release\opencam_vcam.dll` (64 bits) y `target\i686-pc-windows-msvc\release\opencam_vcam.dll` (32 bits).

### Registrar y quitar (como administrador)

```powershell
# 64 bits
C:\Windows\System32\regsvr32.exe "C:\ruta\a\opencam_vcam.dll"
# 32 bits
C:\Windows\SysWOW64\regsvr32.exe "C:\ruta\a\opencam_vcam_x86.dll"
# Quitar: lo mismo con /u
```

El registro escribe el servidor COM en `HKLM\SOFTWARE\Classes\CLSID` (la DLL de 32 bits, en `WOW6432Node`) y añade el filtro a la categoría de cámaras (`{860BB310-5D01-11D0-BD3B-00A0C911CE86}`) con `IFilterMapper2::RegisterFilter`, que guarda también `FilterData` (pin de salida de vídeo YUY2 de captura) e invalida la caché del enumerador de dispositivos. Es lo mismo que hace OBS Virtual Camera.

`windows-vcam/test-register.ps1` registra las dos DLL, comprueba el registro, lista las cámaras DirectShow con ffmpeg y las desregistra. CI lo ejecuta en un runner de Windows.

Si una app ya estaba abierta al instalar (Discord, por ejemplo), ciérrala del todo (también de la bandeja del sistema) y vuelve a abrirla para que vuelva a listar las cámaras.

Después, arranca el cliente y elige **OpenCam Virtual Camera** en la app que use la cámara:

```powershell
cargo run --release
```

En Windows `--device` se ignora.

### Limitaciones

- Formatos YUY2 a 30 fps en 1080p, 720p y 480p (`IAMStreamConfig`); si la memoria compartida trae otra resolución, el filtro escala con letterbox. `SetFormat` no reconecta un pin ya conectado.
- Es una cámara DirectShow, como OBS Virtual Camera: la ven Discord, Zoom, OBS, Chrome/Edge, Teams y la mayoría de apps de escritorio. No es una Media Foundation Virtual Camera, así que no sale en *Configuración > Cámaras* ni en la app Cámara de Windows ni en apps UWP.
- CI registra y lista la cámara en un Windows real, pero no se ha probado con vídeo en una app.

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
