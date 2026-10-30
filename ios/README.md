# OpenCam para iPhone (Fase 2)

App Swift que convierte el iPhone en cámara para el escritorio. Habla el protocolo OPCM v1 de
[`docs/PROTOCOL.md`](../docs/PROTOCOL.md) en el puerto 4747.

## Archivos

- `OpenCam/OPCM.swift`: cabecera de 24 bytes big-endian y tipos de mensaje.
- `OpenCam/OpcmServer.swift`: servidor TCP (Network.framework), HELLO/CAPS, CONFIG, PING/PONG, un cliente a la vez.
- `OpenCam/H264Encoder.swift`: VideoToolbox H.264 en Annex-B (SPS/PPS delante de los keyframes).
- `OpenCam/CameraStreamer.swift`: captura AVFoundation 1080p que arranca al recibir CONFIG.
- `OpenCam/OpenCamApp.swift`: interfaz mínima (iniciar/parar).
- `OpenCam/Info.plist`: permisos de cámara y red local.

## Conexión por USB

El escritorio macOS/Linux usa usbmuxd: `opencam-desktop iphone`. Abre un túnel al puerto 4747 del
iPhone sin necesidad de Wi-Fi.

## Estado de verificación

- **No compilado.** El entorno de desarrollo no tiene Xcode ni Swift, así que el código Swift no se
  ha compilado ni probado.
- El escritorio sí se compila (`cargo build`); el módulo `usbmuxd` se compila solo en Unix.
- Pendiente: crear el proyecto Xcode (no incluido), compilar y probar en un iPhone real con
  `opencam-desktop iphone`.
