# Verifie un PDF avec le moteur PDF de Windows (Windows.Data.Pdf, WinRT) :
# chargement + nombre de pages + rendu de la page 1 en PNG (a regarder).
# Aucune application ouverte, aucun dialogue possible. ASCII pur (piege 2).
param([string]$Pdf, [string]$Png)
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.DataReader, Windows.Storage.Streams, ContentType = WindowsRuntime]
$ext = [System.WindowsRuntimeSystemExtensions].GetMethods()
$asTaskOp = ($ext | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
$asTaskAct = ($ext | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' })[0]
function Attendre($op, [Type]$t) { $task = $asTaskOp.MakeGenericMethod($t).Invoke($null, @($op)); [void]$task.Wait(30000); $task.Result }
function AttendreAction($a) { [void]$asTaskAct.Invoke($null, @($a)).Wait(30000) }
try {
    $file = Attendre ([Windows.Storage.StorageFile]::GetFileFromPathAsync($Pdf)) ([Windows.Storage.StorageFile])
    $doc = Attendre ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
    "OK  $([IO.Path]::GetFileName($Pdf)) (moteur PDF Windows) : pages=$($doc.PageCount)"
    $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
    AttendreAction ($doc.GetPage(0).RenderToStreamAsync($stream))
    $taille = [uint32]$stream.Size
    $lecteur = New-Object Windows.Storage.Streams.DataReader($stream.GetInputStreamAt(0))
    $null = Attendre ($lecteur.LoadAsync($taille)) ([uint32])
    $octets = New-Object byte[] $taille
    $lecteur.ReadBytes($octets)
    [IO.File]::WriteAllBytes($Png, $octets)
    "rendu page 1 : $Png ($taille octets)"
} catch {
    "ECHEC $([IO.Path]::GetFileName($Pdf)) : $($_.Exception.Message)"
}
