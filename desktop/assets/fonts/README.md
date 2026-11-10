# Fuentes de la ventana de escritorio

Subconjuntos (solo los caracteres que usa la interfaz) incrustados en el binario:

| Archivo | Origen | Licencia |
| --- | --- | --- |
| `Inter-Regular.otf`, `Inter-SemiBold.otf` | [Inter](https://github.com/rsms/inter), latín básico y latín-1 | SIL OFL 1.1 (`Inter-OFL.txt`) |
| `icons-noto.ttf` | [Noto Emoji](https://github.com/googlefonts/noto-emoji) monocromo, iconos de la interfaz | SIL OFL 1.1 (`NotoEmoji-OFL.txt`) |
| `icons-extra.ttf` | [emoji-icon-font](https://github.com/jslegers/emoji-icon-font): ⚙ 🎞 ⏶ ⏷ ■ | MIT (`emoji-icon-font-LICENSE.txt`) |

Para añadir un icono, regenera el subconjunto con `pyftsubset` (fonttools) desde las
fuentes de `epaint_default_fonts` y añade el carácter a `--unicodes`.
