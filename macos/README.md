# OpenCam para macOS: cámara virtual

`OpenCam.app` añade la cámara **OpenCam** a macOS con una extensión de cámara de CoreMediaIO
(la forma que Apple soporta desde macOS 12.3; los antiguos plug-ins DAL ya no cargan en
macOS 14). Cualquier app que use la cámara (FaceTime, Zoom, Meet en Safari o Chrome, Teams,
OBS, QuickTime…) la ve como una webcam más.

## Cómo funciona

```
móvil ──OPCM──> opencam-desktop ──YUYV por stdin──> OpenCam.app --feed ──sink──> extensión ──> apps
```

- `CameraExtension/`: la extensión. Publica un dispositivo con dos streams: el de salida
  (lo que ven las apps) y uno de entrada (*sink*). Reenvía el último frame recibido a 30 fps,
  o negro si no llega nada en 2 segundos.
- `OpenCam/`: la app. Abierta normalmente muestra los botones para instalar o quitar la
  extensión. Con `--feed` no abre ventana: lee frames YUYV 1920x1080 de stdin y los mete en el
  stream de entrada. El cliente `opencam-desktop` la lanza solo (sink `macos`, por defecto en
  macOS), así que no hace falta abrirla a mano para emitir.
- `Shared/CameraConfig.swift`: UID del dispositivo y formato, comunes a los dos.

## Compilar e instalar

Requisitos: macOS 13 o posterior, Xcode 15 o posterior y una cuenta de Apple Developer (la de
pago: las extensiones de sistema necesitan el entitlement `system-extension.install`).

```sh
brew install xcodegen
cd macos && xcodegen generate
open OpenCamMac.xcodeproj
```

1. En Xcode, en *Signing & Capabilities* de los dos targets (`OpenCam` y `CameraExtension`),
   elige tu Team. El App Group `<TeamID>.dev.opencam.mac` se rellena solo con tu Team ID.
2. Compila el esquema `OpenCam` (Product › Archive, o Build en Release) y copia `OpenCam.app`
   a **/Applications**. macOS solo instala extensiones desde Aplicaciones.
3. Abre OpenCam.app y pulsa **Instalar cámara**. macOS pide permiso: Ajustes del Sistema ›
   General › Ítems de inicio y extensiones (en macOS 13, Privacidad y seguridad).
4. Arranca el cliente con la dirección del móvil:

   ```sh
   opencam-desktop 127.0.0.1:4747     # tras adb forward tcp:4747 tcp:4747
   opencam-desktop iphone             # iPhone por USB
   ```

5. En la app de vídeo, elige la cámara **OpenCam**. Si ya estaba abierta, reiníciala.

Para quitarla: **Desinstalar** en la app, o `systemextensionsctl uninstall <TeamID> dev.opencam.mac.camera`.

### Sin cuenta de pago (solo desarrollo)

Con SIP desactivado se pueden cargar extensiones sin firma de Apple:
`systemextensionsctl developer on`, y la app puede estar fuera de /Applications. No lo
recomendamos fuera de una máquina de pruebas.

### Si la app está en otro sitio

El cliente busca `/Applications/OpenCam.app` y `~/Applications/OpenCam.app`. Para otra ruta:

```sh
OPENCAM_FEEDER="/ruta/OpenCam.app/Contents/MacOS/OpenCam" opencam-desktop 127.0.0.1:4747
```

## Limitaciones

- Un único formato: 1920x1080 YUYV a 30 fps. Si pides otra resolución al móvil
  (`--width/--height`), el cliente la escala con letterbox a 1080p.
- CI compila la app y la extensión sin firmar en cada PR; nadie la ha instalado ni probado aún
  en un Mac real.
- No hay descarga firmada en las releases: hace falta compilarla con tu cuenta de Apple.
