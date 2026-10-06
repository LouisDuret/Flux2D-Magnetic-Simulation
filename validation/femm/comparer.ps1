# Compare Flux2D à FEMM 4.2 sur les scènes de référence (section 3.9 du document) :
# écrit les scripts Lua, les lance dans FEMM, puis confronte les forces et les inductions.
#
#   .\validation\femm\comparer.ps1                      # FEMM installé dans C:\femm42
#   .\validation\femm\comparer.ps1 -Femm "D:\femm42\bin\femm.exe"
param([string]$Femm = "C:\femm42\bin\femm.exe")

if (-not (Test-Path $Femm)) {
    Write-Error "FEMM introuvable : $Femm. Installez FEMM 4.2 (femm.info) ou passez -Femm <chemin de femm.exe>."
    exit 1
}
$racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Push-Location $racine
try {
    cargo test --release -p flux-solver --test femm scripts_are_written
    foreach ($script in Get-ChildItem (Join-Path $PSScriptRoot "*.lua")) {
        "FEMM : $($script.Name)"
        Start-Process -FilePath $Femm -ArgumentList "-lua-script=`"$($script.FullName)`"", "-windowhide" -Wait
    }
    cargo test --release -p flux-solver --test femm flux2d_matches_femm -- --nocapture
}
finally {
    Pop-Location
}
