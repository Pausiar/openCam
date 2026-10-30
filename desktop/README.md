# OpenCam desktop

Cliente de escritorio: se conecta al móvil por el protocolo OPCM (ver `docs/PROTOCOL.md`), decodifica H.264 y lo entrega a una cámara virtual.

## Estado

| Sistema | Cámara virtual | Estado |
|---|---|---|
| Linux | `v4l2loopback` (`/dev/videoN`) | Implementado. Compila; no probado con el módulo cargado en este entorno. |
| Windows | Filtro DirectShow (`desktop/windows-vcam`) | Implementado. Sin compilar ni probar en Windows todavía. |

## Compilar y probar

```bash
cd desktop
cargo build
cargo test
```

## Windows: cámara virtual DirectShow

Dos piezas:

- El cliente (`desktop/`) escribe cada frame YUY2 1920x1080 en la memoria compartida `Local\OpenCamVirtualCamera`. El formato está en `core/src/shared_frame.rs`.
- El filtro (`desktop/windows-vcam/`, DLL `opencam_vcam.dll`) es una fuente de vídeo DirectShow que lee esa memoria y la entrega a las apps. Si la memoria no existe (el cliente no está en marcha), entrega negro; si el cliente se detiene, repite el último frame. Reabre la memoria cada segundo, así que el cliente puede arrancar o reiniciarse después de la cámara.

El filtro es un crate independiente: `cargo build` en `desktop/` no lo compila.

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

En Windows el segundo argumento (dispositivo) se ignora.

### Limitaciones

- Un único formato: YUY2 1920x1080 a 30 fps. No implementa `IAMStreamConfig` ni `FilterData`, así que no hay selector de resolución.
- El pin no informa el filtro al que pertenece (`pFilter` nulo en `PIN_INFO`).
- No es Media Foundation Virtual Camera; las apps que sólo listan cámaras con Media Foundation no la verán.
- Sin compilar ni registrar en Windows todavía: el código no se ha probado.

## Linux: preparar la cámara virtual

```bash
sudo modprobe v4l2loopback video_nr=10 card_label="OpenCam" exclusive_caps=1
```

## Uso con el móvil por USB

```bash
adb forward tcp:4747 tcp:4747
cargo run -- 127.0.0.1:4747 /dev/video10
```

El primer argumento es la dirección del móvil y el segundo, el dispositivo de salida.

### iPhone por USB (macOS/Linux)

Requiere usbmuxd (en macOS viene con el sistema) y la app iOS en primer plano. No necesita `adb`:

```bash
cargo run -- iphone /dev/video10
```
