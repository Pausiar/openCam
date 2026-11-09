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
| `0x04` | `AUTH`            | ambos sentidos     | 32 bytes: HMAC de confirmación (ver «Seguridad») |
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

1. El escritorio abre TCP y envía `HELLO` con
   `{"proto":1,"platform":"desktop","name":"...","secure":1,"client_id":"<hex>","pub":"<hex>","auth":"code|key"}`.
2. El móvil responde `HELLO` con `{"proto":1,"platform":"android","secure":1,"phone_id":"<hex>","pub":"<hex>","auth":"code|key"}`,
   ambos lados intercambian `AUTH` (ver «Seguridad») y a partir de ahí todo va cifrado. El móvil envía `CAPS`:
   ```json
   {"platform":"android","codecs":["h264","h265"],
    "resolutions":[[1920,1080],[3840,2160]],
    "fps":[15,24,30,60],"lenses":["wide","ultrawide","tele"]}
   ```
3. El escritorio envía `CONFIG`:
   ```json
   {"codec":"h264","width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"wide"}
   ```
   Campos avanzados opcionales (si faltan, el móvil conserva su valor actual, p. ej. lo elegido en su pantalla):

   | Campo            | Valores | Efecto |
   |------------------|---------|--------|
   | `facing`         | `"back"` \| `"front"` | Cámara trasera o frontal. Si no hay frontal, se usa la trasera |
   | `stabilization`  | `true` \| `false` | Estabilización de vídeo (EIS y/o OIS si existen) |
   | `zoom`           | número ≥ 1 | Factor de zoom; el móvil lo limita a `max_zoom` |
   | `focus`          | `"auto"` \| `"manual"` | Autoenfoque continuo o enfoque manual |
   | `focus_distance` | 0.0 – 1.0 | Con `focus:"manual"`: 0 = infinito, 1 = lo más cerca que permita la lente |

   `CONFIG` puede reenviarse durante la sesión. `zoom`, `focus`, `focus_distance`, `stabilization` y `bitrate_kbps`
   se aplican en caliente; el resto (resolución, fps, códec, lente, `facing`) reinicia cámara y codificador, y el
   primer frame vuelve a ser keyframe. El **modo espejo** no viaja en el protocolo: lo aplica el escritorio.

   En `CAPS` el móvil puede anunciar además `"facings":["back","front"]`, `"max_zoom":N`,
   `"stabilization":bool` y `"manual_focus":bool`.
4. El móvil empieza a emitir `VIDEO`. El primer frame debe ser keyframe con SPS/PPS.
5. El escritorio manda `PING` cada 1 s. Latencia = `ahora − timestamp_us` del `PONG`.

## Seguridad

El móvil es servidor en la red local, así que sin protección cualquiera en la misma Wi-Fi
podría ver la cámara. Por eso cada conexión se empareja y se cifra.

**Emparejamiento.** El móvil muestra un código de 6 dígitos. La primera vez el usuario lo
escribe en el PC (`--code` o la interfaz). Después ambos guardan una clave de 32 bytes
(`pairing_key`) y las reconexiones no piden nada. El código cambia tras cada emparejamiento
correcto y tras 5 intentos fallidos seguidos, así que adivinarlo probando no es viable.

**Handshake** (en claro, payload máximo 64 KiB y 10 s de plazo; antes de terminarlo una
conexión nueva no puede expulsar al PC conectado):

1. `HELLO` escritorio → móvil: `client_id` (id persistente del PC), `pub` (clave pública P-256
   efímera sin comprimir, 65 bytes en hex) y `auth`: `"code"` si el usuario tecleó un código,
   `"key"` si el PC quiere usar la clave guardada.
2. `HELLO` móvil → escritorio: `phone_id`, `pub` y `auth`. El móvil responde `"key"` solo si el
   PC pidió `key` y tiene clave guardada para ese `client_id`; si no, `"code"`.
3. Ambos calculan:
   - `psk` = clave guardada, o `SHA-256("opencam-pair-code:" || código)`.
   - `th` = `SHA-256(u32be(len(H_d)) || H_d || u32be(len(H_p)) || H_p)`, con `H_d`/`H_p` los
     payloads exactos de los dos `HELLO`.
   - `okm` = `HKDF-SHA256(ikm = ECDH(x), salt = psk, info = "opencam-v1" || th, 128 bytes)`
     → `k_d2p | k_p2d | k_confirm | pairing_key` (32 bytes cada una).
4. `AUTH` escritorio → móvil: `HMAC-SHA256(k_confirm, "desktop")`. Si no cuadra, el móvil
   responde `ERROR` `bad_code` (o `bad_key`) y cierra.
5. `AUTH` móvil → escritorio: `HMAC-SHA256(k_confirm, "phone")`. El escritorio comprueba así
   que habla con el móvil que mostró el código.

**Cifrado.** Desde el mensaje siguiente al `AUTH` de cada lado, cada payload es
`AES-256-GCM(k_dir, nonce = 0⁴ || u64be(contador), aad = cabecera de 24 bytes)` con un
contador por sentido que empieza en 0. La cabecera va en claro (para el framing) pero
autenticada, y `payload_len` incluye la etiqueta de 16 bytes. Un mensaje alterado, repetido,
reordenado o sin cifrar no descifra y la conexión se cierra.

**Compatibilidad.** Un móvil que recibe un `HELLO` sin `"secure":1` responde `ERROR`
`pairing_required` y cierra. Un escritorio que recibe un `HELLO` sin `secure` (app antigua)
sigue sin cifrar y lo avisa.

**Límites conocidos.** El primer emparejamiento con código de 6 dígitos resiste a un
espía pasivo, pero un atacante activo que haga de intermediario *en ese momento* podría
probar los 10⁶ códigos sin conexión a partir del `AUTH`. Las conexiones posteriores usan la
clave de 256 bits y no tienen ese problema. Tipos de mensaje, tamaños y marcas de tiempo
viajan en claro.

Los vectores de prueba de `core/src/secure.rs` (`tests::vector`) se repiten en
`android/app/src/test/.../SecureTest.kt` para garantizar que las implementaciones coinciden.

## Reglas de robustez

- **Pérdida o corrupción**: si el parser encuentra un `magic` inválido, cierra la conexión y reconecta. No intenta resincronizar.
- **Recuperación**: si el decodificador pierde referencias, envía `KEYFRAME_REQUEST`.
- **Cola limitada**: el receptor mantiene una cola acotada. Si se llena, descarta los frames más antiguos. Un keyframe nuevo descarta los P-frames anteriores, porque ya no son útiles para la baja latencia.
- **Arranque**: el decodificador descarta cualquier P-frame hasta el primer keyframe.
- **Tamaño**: `payload_len > 16 MiB` es error de protocolo.
- **Versión**: si `version` no es 1, el receptor responde `ERROR` con código `unsupported_version` y cierra.

## Estimación de ancho de banda

Un 4K30 H.265 a 20 Mbps ocupa unos 2.5 MB/s. Un frame clave 4K puede superar 500 KB; por eso el límite de payload es 16 MiB, muy por encima de lo necesario.
