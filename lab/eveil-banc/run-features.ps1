# Extraction des features (GATE 1b) - ASCII pur (piege PS 5.1 / UTF-8).
# Split PAR VOIX : validation = timbres JAMAIS vus a l'entrainement
# (developpeuse Pocket + clone-jessica) -> mesure la generalisation.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force feats | Out-Null

$exe = ".\eveil-banc.exe"

# --- entrainement ---
& $exe features corpus\pos\fabien        feats\tr-pos-fabien.f32   word 8 101
& $exe features corpus\pos\clone-pierre  feats\tr-pos-cpierre.f32  word 8 102
& $exe features corpus\pos\piper-pierre  feats\tr-pos-ppierre.f32  word 12 103
& $exe features corpus\pos\piper-jessica feats\tr-pos-pjessica.f32 word 12 104

& $exe features corpus\neg-mots\fabien        feats\tr-neg-fabien.f32   word 6 111
& $exe features corpus\neg-mots\clone-pierre  feats\tr-neg-cpierre.f32  word 6 112
& $exe features corpus\neg-mots\piper-pierre  feats\tr-neg-ppierre.f32  word 8 113
& $exe features corpus\neg-mots\piper-jessica feats\tr-neg-pjessica.f32 word 8 114
& $exe features corpus\neg-phrases\fabien        feats\tr-phr-fabien.f32   slide
& $exe features corpus\neg-phrases\clone-pierre  feats\tr-phr-cpierre.f32  slide
& $exe features corpus\neg-phrases\piper-pierre  feats\tr-phr-ppierre.f32  slide
& $exe features corpus\neg-phrases\piper-jessica feats\tr-phr-pjessica.f32 slide
& $exe bruit feats\tr-bruit.f32 600 121

# --- validation (voix tenues) ---
& $exe features corpus\pos\developpeuse   feats\va-pos-dev.f32      word 6 201
& $exe features corpus\pos\clone-jessica  feats\va-pos-cjessica.f32 word 6 202
& $exe features corpus\neg-mots\developpeuse  feats\va-neg-dev.f32      word 6 211
& $exe features corpus\neg-mots\clone-jessica feats\va-neg-cjessica.f32 word 6 212
& $exe features corpus\neg-phrases\developpeuse  feats\va-phr-dev.f32      slide
& $exe features corpus\neg-phrases\clone-jessica feats\va-phr-cjessica.f32 slide
& $exe bruit feats\va-bruit.f32 200 221

Write-Output "features OK"
