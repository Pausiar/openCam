# Prueba de registro de la cámara virtual en un Windows real (CI o a mano, como administrador).
#
#   pwsh -File test-register.ps1 -Dll64 ruta\opencam_vcam.dll -Dll32 ruta\opencam_vcam_x86.dll
#
# Registra las dos DLL con su regsvr32, comprueba que cada vista del registro (64 y 32 bits)
# tiene el servidor COM y la entrada en la categoría de cámaras con FilterData, que
# ffmpeg (DirectShow, como Discord) lista "OpenCam Virtual Camera", y que al desregistrar
# no queda nada.
param(
    [Parameter(Mandatory = $true)][string]$Dll64,
    [Parameter(Mandatory = $true)][string]$Dll32
)
$ErrorActionPreference = 'Stop'

$clsid = '{6F1C2A4E-8B3D-4E2A-9C51-0D7A3B6E9F21}'
$category = '{860BB310-5D01-11D0-BD3B-00A0C911CE86}'
$name = 'OpenCam Virtual Camera'
$views = @(
    @{ Bits = 64; Dll = (Resolve-Path $Dll64).Path; Regsvr = "$env:WINDIR\System32\regsvr32.exe"; Root = 'HKLM:\SOFTWARE\Classes\CLSID' },
    @{ Bits = 32; Dll = (Resolve-Path $Dll32).Path; Regsvr = "$env:WINDIR\SysWOW64\regsvr32.exe"; Root = 'HKLM:\SOFTWARE\WOW6432Node\Classes\CLSID' }
)

function Fail($msg) { Write-Host "ERROR: $msg"; exit 1 }

function Regsvr($view, [string[]]$extra) {
    $p = Start-Process -FilePath $view.Regsvr -ArgumentList ($extra + @('/s', "`"$($view.Dll)`"")) -Wait -PassThru
    if ($p.ExitCode -ne 0) { Fail "regsvr32 $($view.Bits) bits $extra salió con $($p.ExitCode)" }
}

foreach ($v in $views) {
    Regsvr $v @()
    $server = Get-ItemProperty "$($v.Root)\$clsid\InprocServer32"
    if ($server.'(default)' -ne $v.Dll) { Fail "$($v.Bits) bits: InprocServer32 = '$($server.'(default)')'" }
    if ($server.ThreadingModel -ne 'Both') { Fail "$($v.Bits) bits: ThreadingModel = '$($server.ThreadingModel)'" }
    $inst = Get-ItemProperty "$($v.Root)\$category\Instance\$clsid"
    if ($inst.FriendlyName -ne $name) { Fail "$($v.Bits) bits: FriendlyName = '$($inst.FriendlyName)'" }
    if ($inst.CLSID -ne $clsid) { Fail "$($v.Bits) bits: CLSID = '$($inst.CLSID)'" }
    if (-not $inst.FilterData -or $inst.FilterData.Length -lt 16) { Fail "$($v.Bits) bits: sin FilterData" }
    Write-Host "OK: registro de $($v.Bits) bits"
}

# Enumeración real por DirectShow (lo mismo que hacen Discord, Zoom u OBS al listar cámaras).
$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
if ($ffmpeg) {
    $out = & $ffmpeg.Source -hide_banner -list_devices true -f dshow -i dummy 2>&1 | Out-String
    Write-Host $out
    if ($out -notmatch [regex]::Escape($name)) { Fail "ffmpeg no lista '$name'" }
    Write-Host 'OK: ffmpeg (DirectShow, 64 bits) lista la cámara'
} else {
    Write-Host 'AVISO: no hay ffmpeg; se omite la enumeración por DirectShow'
}

foreach ($v in $views) {
    Regsvr $v @('/u')
    if (Test-Path "$($v.Root)\$clsid") { Fail "$($v.Bits) bits: el CLSID sigue tras desregistrar" }
    if (Test-Path "$($v.Root)\$category\Instance\$clsid") { Fail "$($v.Bits) bits: la instancia sigue tras desregistrar" }
    Write-Host "OK: desregistro de $($v.Bits) bits"
}
