# Enrollment « Waly » (R6b) - prises REELLES de Michee - ASCII pur (piege 2).
# A lancer A LA MAIN dans une console (les prises se font au micro).
# Usage : powershell -ExecutionPolicy Bypass -File run-enroll.ps1 [-N 30]
param([int]$N = 30)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$voice = "C:\waly\bin\waly-voice.exe"
$dir = "corpus\enroll\michee"
New-Item -ItemType Directory -Force $dir | Out-Null

Write-Output "Enrollment : $N prises de « Waly » (2 s chacune, 16 kHz)."
Write-Output "Varie les conditions : pres du micro ET a 3 m, fort et doux,"
Write-Output "appel (Waly !), question (Waly ?), au milieu d'une phrase (dis Waly)."
Write-Output ""
for ($i = 1; $i -le $N; $i++) {
  Read-Host "Prise $i/$N - Entree, puis dis « Waly » pendant l'enregistrement" | Out-Null
  $out = Join-Path $dir ("waly_{0:d3}.wav" -f $i)
  & $voice record $out 2
}
Write-Output ""
Write-Output "=== Validation : le modele du banc detecte-t-il TA voix ? ==="
.\eveil-banc.exe frr $dir waly_v4.onnx 0.85
.\eveil-banc.exe frr $dir waly_v4.onnx 0.9
Write-Output ""
Write-Output "FRR haut ? Fine-tuning local (les prises ENTRENT a l'entrainement) :"
Write-Output "  .\eveil-banc.exe features corpus\enroll\michee feats\enroll-michee.f32 word 12 701"
Write-Output "  puis (WSL) : re-lancer train_waly.py en ajoutant feats/enroll-michee.f32"
Write-Output "  a --pos, exporter waly_wake.onnx, recopier vers"
Write-Output "  C:\waly\engines\models\openwakeword\waly_wake.onnx"
