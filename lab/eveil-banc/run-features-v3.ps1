# Extraction v3 (GATE 1b, pseudo-timbres par pitch-shift) - ASCII pur.
# Split par FAMILLE : developpeuse (base + p088 + p114) = validation,
# jamais vue ; clone-jessica base sort de la validation (ses variantes
# entrent a l'entrainement -> timbre adjacent = fuite).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$exe = ".\eveil-banc.exe"

foreach ($v in @("fabien-p088","fabien-p114","clone-pierre-p088","clone-pierre-p114","clone-jessica-p088","clone-jessica-p114")) {
  & $exe features corpus\pos\$v      feats\tr3-pos-$v.f32    word 8 501
  & $exe features corpus\pos-ctx\$v  feats\tr3-posctx-$v.f32 word 8 502
  & $exe features corpus\neg-mots\$v feats\tr3-neg-$v.f32    word 6 503
  & $exe features corpus\neg-pieges\$v  feats\tr3-piege-$v.f32 slide 2 504
  & $exe features corpus\neg-phrases\$v feats\tr3-phr-$v.f32   slide 2 505
}
foreach ($v in @("developpeuse-p088","developpeuse-p114")) {
  & $exe features corpus\pos\$v      feats\va3-pos-$v.f32    word 6 601
  & $exe features corpus\pos-ctx\$v  feats\va3-posctx-$v.f32 word 6 602
  & $exe features corpus\neg-mots\$v feats\va3-neg-$v.f32    word 6 603
  & $exe features corpus\neg-pieges\$v  feats\va3-piege-$v.f32 slide 1 604
  & $exe features corpus\neg-phrases\$v feats\va3-phr-$v.f32   slide 1 605
}
Write-Output "features v3 OK"
