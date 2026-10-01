# Extraction v2 (GATE 1b, iteration) - ASCII pur (piege PS 5.1).
# Ajouts : porteuses positives (pos-ctx), phrases pieges (neg-pieges, slide),
# phrases re-extraites avec passes bruitees (slide K=3).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$exe = ".\eveil-banc.exe"

# --- entrainement : porteuses ---
& $exe features corpus\pos-ctx\fabien        feats\tr-posctx-fabien.f32   word 8 301
& $exe features corpus\pos-ctx\clone-pierre  feats\tr-posctx-cpierre.f32  word 8 302
& $exe features corpus\pos-ctx\piper-pierre  feats\tr-posctx-ppierre.f32  word 10 303
& $exe features corpus\pos-ctx\piper-jessica feats\tr-posctx-pjessica.f32 word 10 304

# --- entrainement : phrases pieges (slide bruite) ---
& $exe features corpus\neg-pieges\fabien        feats\tr-piege-fabien.f32   slide 3 311
& $exe features corpus\neg-pieges\clone-pierre  feats\tr-piege-cpierre.f32  slide 3 312
& $exe features corpus\neg-pieges\piper-pierre  feats\tr-piege-ppierre.f32  slide 3 313
& $exe features corpus\neg-pieges\piper-jessica feats\tr-piege-pjessica.f32 slide 3 314

# --- entrainement : phrases normales re-extraites avec passes bruitees ---
& $exe features corpus\neg-phrases\fabien        feats\tr-phr-fabien.f32   slide 3 321
& $exe features corpus\neg-phrases\clone-pierre  feats\tr-phr-cpierre.f32  slide 3 322
& $exe features corpus\neg-phrases\piper-pierre  feats\tr-phr-ppierre.f32  slide 3 323
& $exe features corpus\neg-phrases\piper-jessica feats\tr-phr-pjessica.f32 slide 3 324

# --- validation (voix tenues, propre = miroir du runtime) ---
& $exe features corpus\pos-ctx\developpeuse   feats\va-posctx-dev.f32      word 6 401
& $exe features corpus\pos-ctx\clone-jessica  feats\va-posctx-cjessica.f32 word 6 402
& $exe features corpus\neg-pieges\developpeuse  feats\va-piege-dev.f32      slide 1 411
& $exe features corpus\neg-pieges\clone-jessica feats\va-piege-cjessica.f32 slide 1 412

Write-Output "features v2 OK"
