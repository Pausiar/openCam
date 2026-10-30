# OpenCam desktop

Cliente de escritorio: se conecta al móvil por el protocolo OPCM (ver `docs/PROTOCOL.md`), decodifica H.264 y lo entrega a una cámara virtual.

## Estado

| Sistema | Cámara virtual | Estado |
|---|---|---|
| Linux | `v4l2loopback` (`/dev/videoN`) | Implementado. Compila; no probado con el módulo cargado en este entorno. |
| Windows | MF Virtual Camera (Win11) / DirectShow (Win10) | Pendiente. |

## Compilar y probar

```bash
cd desktop
cargo build
cargo test
```

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
