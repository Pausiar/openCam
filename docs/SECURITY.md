# Seguridad de OpenCam

Revisión de noviembre de 2026. El detalle del protocolo está en `PROTOCOL.md`, sección «Seguridad».

## Modelo de amenazas

El móvil abre un servidor TCP (puerto 4747) y se anuncia por mDNS en la red local. El activo que
hay que proteger es la imagen de la cámara. El atacante típico está en la misma Wi-Fi (cafetería,
oficina, residencia) o es otra app del mismo móvil.

## Problemas encontrados y estado

| # | Problema | Gravedad | Estado |
|---|----------|----------|--------|
| 1 | Cualquiera en la red podía conectarse al 4747 y recibir el vídeo de la cámara, sin código ni aviso. | Alta | Corregido: emparejamiento obligatorio por código de 6 dígitos y clave guardada. |
| 2 | El vídeo y los ajustes viajaban en claro: un espía en la Wi-Fi veía la imagen. | Alta | Corregido: ECDH P-256 + AES-256-GCM, con la cabecera autenticada. |
| 3 | Android: una conexión nueva expulsaba al PC conectado antes de cualquier comprobación (cualquiera podía cortar la sesión). | Media | Corregido: solo sustituye al cliente tras autenticarse. |
| 4 | iOS: una conexión muda ocupaba el único hueco para siempre y bloqueaba al PC legítimo. | Media | Corregido: plazo de 10 s para el handshake y hasta 4 handshakes en paralelo. |
| 5 | Antes de autenticarse, un mensaje podía hacer reservar 16 MiB por conexión. | Baja | Corregido: 64 KiB máximo hasta terminar el handshake (Android, iOS y escritorio). |
| 6 | Android aceptaba cualquier valor en CONFIG (tamaños negativos o enormes, códec o lente arbitrarios) y podía tumbar el codificador. | Baja | Corregido: valores acotados y listas blancas (`StreamConfig.parse`). El CLI del escritorio también acota. |
| 7 | El JSON de ERROR en Android escapaba a mano y no cubría caracteres de control. | Baja | Corregido: se genera con `JSONObject`. |
| 8 | Las copias de seguridad de Android podían sacar del teléfono las claves de emparejamiento. | Baja | Corregido: `allowBackup="false"`. En iOS las claves van al llavero con `ThisDeviceOnly`. |
| 9 | Las releases no publicaban sumas de comprobación. | Baja | Corregido: `SHA256SUMS` en cada release. |
| 10 | El código de 6 dígitos rotaba cada 5 fallos pero sin pausa: alguien en la misma red podía probar códigos sin parar (cada intento acierta 1 de cada 10⁶) y emparejarse en unas horas. | Media | Corregido: cada tanda de 5 fallos bloquea el emparejamiento por código 30 s, duplicándose hasta 10 min (Android e iOS). Los PCs ya emparejados no se ven afectados. |

## Riesgos que quedan

- **Primer emparejamiento ante un intermediario activo.** Con un código de 6 dígitos no se puede
  evitar que alguien que intercepta y modifica el tráfico *justo durante el primer emparejamiento*
  pruebe todos los códigos sin conexión. Contra un espía pasivo sí protege. La solución completa
  es un PAKE (p. ej. SPAKE2) o un QR con 128 bits de secreto escaneado por el móvil; queda
  pendiente para cuando haya interfaz de escritorio con QR.
- **Degradación con apps antiguas.** El escritorio acepta móviles sin cifrado (apps anteriores a
  este cambio) y lo avisa. Un intermediario podría hacerse pasar por un móvil antiguo y enviar
  vídeo falso, pero no puede ver la cámara real (el móvil nuevo exige emparejamiento).
- **Metadatos en claro.** Tipos de mensaje, tamaños y marcas de tiempo no se cifran.
- **Memoria compartida en Windows.** `Local\OpenCamVirtualCamera` es accesible para procesos de la
  misma sesión de usuario (es lo que necesita la cámara virtual). No es un límite de seguridad.
- **Acciones de GitHub** fijadas por etiqueta (`@v4`), no por SHA. Fijarlas por SHA protege de
  una etiqueta movida en un proyecto de terceros.

## Tests

- `core/src/secure.rs`: acuerdo de claves, código incorrecto, manipulación, repetición,
  reordenación, claves públicas inválidas y un vector fijo.
- `android/.../SecureTest.kt`: el mismo vector (interoperabilidad Rust ↔ Android), HKDF contra la
  RFC 5869 y rotación del código.
- `android/.../OpcmTest.kt`: servidor real con un escritorio de prueba: emparejamiento, clave
  guardada, código incorrecto, extraño que no puede expulsar al cliente, payload grande antes de
  autenticar, mensaje manipulado y CONFIG fuera de rango.
- `desktop/src/client.rs`: cliente real contra un móvil de prueba: emparejar y reconectar con la
  clave, código pedido o incorrecto, móvil impostor y límite de payload.
- iOS no tiene target de tests; usa las mismas primitivas (CryptoKit) que los vectores fijan.
