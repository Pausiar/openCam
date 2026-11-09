# Protocolo OpenCam v1

Protocolo binario propio, común a Android, iOS y escritorio.

## Transporte

- **Quién escucha**: el móvil es servidor TCP; el escritorio es cliente.
- **USB Android**: el escritorio ejecuta `adb forward tcp:<puerto_local> tcp:4747`.
- **USB iPhone**: el escritorio se conecta a usbmuxd, que reenvía al puerto 4747 de la app.
- **Wi-Fi**: conexión TCP directa a la IP del móvil (descubrimiento mDNS/Bonjour en Fase 4).
- **Bluetooth Android (Fase 4)**: el mismo framing sobre RFCOMM. iPhone no transmite vídeo por Bluetooth.

Puerto por defecto: **4747**. Todos los enteros son **big-endian**.

## Cabecera fija (24 bytes)

| Offset | Tamaño | Campo        | Descripción |
|--------|--------|--------------|-------------|
| 0      | 4      | `magic`      | ASCII `OPCM` (`0x4F 0x50 0x43 0x4D`) |
| 4      | 1      | `version`    | `1` |
| 5      | 1      | `msg_type`   | Ver tabla de tipos |
| 6      | 2      | `flags`      | Bit 0 = keyframe (IDR). Resto reservado = 0 |
| 8      | 4      | `seq`        | Contador por emisor, incrementa en cada mensaje |
| 12     | 8      | `timestamp_us` | Marca de tiempo en µs del emisor (captura en VIDEO, eco en PONG) |
| 20     | 4      | `payload_len`  | Bytes del payload. Máximo 16 MiB |

Tras la cabecera va el payload de `payload_len` bytes.

## Tipos de mensaje

| Código | Nombre            | Dirección          | Payload |
|--------|-------------------|--------------------|---------|
| `0x01` | `HELLO`           | ambos sentidos     | JSON UTF-8 |
| `0x02` | `CAPS`            | móvil → escritorio | JSON UTF-8 |
| `0x03` | `CONFIG`          | escritorio → móvil | JSON UTF-8 |
| `0x10` | `VIDEO`           | móvil → escritorio | Unidad de acceso H.264 Annex-B |
| `0x20` | `PING`            | escritorio → móvil | vacío; `timestamp_us` = reloj local |
| `0x21` | `PONG`            | móvil → escritorio | vacío; devuelve el `timestamp_us` del PING |
| `0x22` | `STATS`           | escritorio → móvil | JSON `{"rtt_ms":N,"dropped":N,"decoded_fps":N,"target_bitrate_kbps":N}` |
| `0x30` | `KEYFRAME_REQUEST`| escritorio → móvil | vacío |
| `0x40` | `AUDIO`           | móvil → escritorio | Reservado (Fase 3) |
| `0x7F` | `ERROR`           | ambos sentidos     | JSON `{"code":"...","message":"..."}` |

Un tipo desconocido se ignora y se registra; no cierra la conexión. Así, un móvil antiguo ignora `STATS`.

`STATS` se envía cada ~1 s: `rtt_ms` es el RTT suavizado (EWMA), `dropped` los frames descartados en la última ventana, `decoded_fps` los frames decodificados por segundo y `target_bitrate_kbps` el bitrate que el escritorio pide al emisor (controlador AIMD: ×0.8 ante congestión, +250 kbps tras varias ventanas estables). El móvil debe reconfigurar el codificador a ese valor.

## Flujo de sesión

1. El escritorio abre TCP y envía `HELLO` con `{"proto":1,"platform":"desktop","name":"..."}`.
2. El móvil responde `HELLO` y después `CAPS`:
   ```json
   {"platform":"android","codecs":["h264","h265"],
    "resolutions":[[1920,1080],[3840,2160]],
    "fps":[15,24,30,60],"lenses":["wide","ultrawide","tele"]}
   ```
   `current` (opcional) lleva los ajustes elegidos en la app del móvil:
   `{"width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"wide"}`.
3. El escritorio envía `CONFIG`:
   ```json
   {"codec":"h264","width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"wide"}
   ```
   Todos los campos son opcionales: el móvil usa sus ajustes locales (`current`) para los que falten. Con «usar la calidad del móvil» el escritorio solo envía `{"codec":"h264"}`.
4. El móvil empieza a emitir `VIDEO`. El primer frame debe ser keyframe con SPS/PPS.
5. El escritorio manda `PING` cada 1 s. Latencia = `ahora − timestamp_us` del `PONG`.

## Reglas de robustez

- **Pérdida o corrupción**: si el parser encuentra un `magic` inválido, cierra la conexión y reconecta. No intenta resincronizar.
- **Recuperación**: si el decodificador pierde referencias, envía `KEYFRAME_REQUEST`.
- **Cola limitada**: el receptor mantiene una cola acotada. Si se llena, descarta los frames más antiguos. Un keyframe nuevo descarta los P-frames anteriores, porque ya no son útiles para la baja latencia.
- **Arranque**: el decodificador descarta cualquier P-frame hasta el primer keyframe.
- **Tamaño**: `payload_len > 16 MiB` es error de protocolo.
- **Versión**: si `version` no es 1, el receptor responde `ERROR` con código `unsupported_version` y cierra.

## Estimación de ancho de banda

Un 4K30 H.265 a 20 Mbps ocupa unos 2.5 MB/s. Un frame clave 4K puede superar 500 KB; por eso el límite de payload es 16 MiB, muy por encima de lo necesario.
