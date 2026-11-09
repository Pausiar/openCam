#!/bin/sh
# Prepara la cámara virtual de OpenCam en Linux (v4l2loopback) y, si se le pasa,
# instala el cliente en /usr/local/bin.
#
#   sudo ./install.sh                       # módulo + configuración persistente
#   sudo ./install.sh ./opencam-desktop     # además copia el cliente a /usr/local/bin
#   sudo ./install.sh --uninstall           # quita la configuración y el cliente
#
# Crea /dev/video10 con el nombre "OpenCam" ahora y en cada arranque.
set -eu

VIDEO_NR=10
LABEL="OpenCam"
MODPROBE_CONF=/etc/modprobe.d/opencam.conf
MODULES_CONF=/etc/modules-load.d/opencam.conf
BIN=/usr/local/bin/opencam-desktop

say() { printf '%s\n' "$*"; }
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "ejecútalo con sudo: sudo $0 $*"

if [ "${1:-}" = "--uninstall" ]; then
    modprobe -r v4l2loopback 2>/dev/null || say "Aviso: v4l2loopback está en uso; se quitará al reiniciar."
    rm -f "$MODPROBE_CONF" "$MODULES_CONF" "$BIN"
    say "Configuración de OpenCam eliminada. El paquete v4l2loopback sigue instalado."
    exit 0
fi

install_module() {
    if modinfo v4l2loopback >/dev/null 2>&1; then
        return 0
    fi
    say "Instalando v4l2loopback…"
    if command -v apt-get >/dev/null 2>&1; then
        apt-get update -qq
        apt-get install -y v4l2loopback-dkms "linux-headers-$(uname -r)" || apt-get install -y v4l2loopback-dkms
    elif command -v dnf >/dev/null 2>&1; then
        # En Fedora el paquete está en RPM Fusion (free).
        dnf install -y v4l2loopback || die "activa RPM Fusion free (https://rpmfusion.org/Configuration) y repite"
    elif command -v pacman >/dev/null 2>&1; then
        pacman -S --needed --noconfirm v4l2loopback-dkms linux-headers || pacman -S --needed --noconfirm v4l2loopback-dkms
    elif command -v zypper >/dev/null 2>&1; then
        zypper --non-interactive install v4l2loopback-kmp-default
    else
        die "no sé instalar v4l2loopback en esta distribución; instálalo a mano y repite"
    fi
    modinfo v4l2loopback >/dev/null 2>&1 || die "v4l2loopback se instaló pero el kernel no lo encuentra; reinicia y repite"
}

install_module

# Persistente: se carga en cada arranque con el mismo número y nombre.
printf 'options v4l2loopback video_nr=%s card_label="%s" exclusive_caps=1\n' "$VIDEO_NR" "$LABEL" > "$MODPROBE_CONF"
printf 'v4l2loopback\n' > "$MODULES_CONF"

# Ahora: recarga el módulo con esas opciones (falla si otra app usa un v4l2loopback).
if lsmod | grep -q '^v4l2loopback'; then
    modprobe -r v4l2loopback 2>/dev/null || say "Aviso: v4l2loopback ya estaba cargado y en uso; las opciones de OpenCam se aplicarán al reiniciar."
fi
if ! modprobe v4l2loopback; then
    if [ -d /sys/firmware/efi ] && command -v mokutil >/dev/null 2>&1 && mokutil --sb-state 2>/dev/null | grep -q enabled; then
        die "el kernel rechazó el módulo. Con Secure Boot activo hay que firmar el módulo DKMS (sigue las instrucciones de tu distribución para registrar la clave MOK) y reiniciar"
    fi
    die "no se pudo cargar v4l2loopback (mira 'dmesg')"
fi

if [ -n "${1:-}" ]; then
    [ -f "$1" ] || die "no existe el cliente: $1"
    install -m 0755 "$1" "$BIN"
    say "Cliente instalado en $BIN"
fi

dev="/dev/video$VIDEO_NR"
if [ -e "$dev" ]; then
    say "Cámara virtual lista: $dev ($LABEL)."
else
    say "Aviso: no aparece $dev; mira 'ls /sys/class/video4linux' para ver dónde quedó."
fi
user="${SUDO_USER:-}"
if [ -n "$user" ] && ! id -nG "$user" | tr ' ' '\n' | grep -qx video; then
    say "Si no puedes abrir $dev, añade tu usuario al grupo video: sudo usermod -aG video $user (y vuelve a iniciar sesión)."
fi
say "Uso: opencam-desktop 127.0.0.1:4747   (tras 'adb forward tcp:4747 tcp:4747'), o la IP del móvil."
