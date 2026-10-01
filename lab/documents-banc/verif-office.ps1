# Verifie des documents generes par Waly en les faisant OUVRIR par le vrai
# Office (lecture seule, fenetre invisible, hors fichiers recents).
# Ne quitte une application QUE si ce script l'a lancee (les instances de
# l'utilisateur restent intactes). ASCII pur (piege 2).
param([string[]]$Fichiers)
$M = [Type]::Missing
function Pids($n) { @(Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }
function Cree($n, $avant) { @(Pids $n | Where-Object { $avant -notcontains $_ }).Count -gt 0 }

foreach ($f in $Fichiers) {
    $ext = [IO.Path]::GetExtension($f).ToLower()
    $nom = [IO.Path]::GetFileName($f)
    try {
        # PDF : PAS via Word (dialogue de conversion invisible = blocage,
        # vecu 2026-09-10) -> verif-pdf.ps1 (moteur PDF de Windows).
        if ($ext -eq '.docx') {
            $avant = Pids 'WINWORD'
            $app = New-Object -ComObject Word.Application
            $cree = Cree 'WINWORD' $avant
            $alertes = $app.DisplayAlerts
            $app.DisplayAlerts = 0
            try {
                # Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles,
                #      PwdDoc, PwdTpl, Revert, WPwdDoc, WPwdTpl, Format, Encoding, Visible)
                $doc = $app.Documents.Open($f, $false, $true, $false, $M, $M, $M, $M, $M, $M, $M, $false)
                $p1 = $doc.Paragraphs.Item(1)
                $texte = $doc.Range().Text -replace "[\r\n\a]+", ' | '
                "OK  $nom (Word) : paragraphes=$($doc.Paragraphs.Count) tableaux=$($doc.Tables.Count) listes=$($doc.Lists.Count) style1='$($p1.Style.NameLocal)' texte='$($texte.Substring(0, [Math]::Min(110, $texte.Length)))'"
                $doc.Close(0)
            } finally {
                $app.DisplayAlerts = $alertes
                if ($cree) { $app.Quit() }
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
            }
        } elseif ($ext -eq '.xlsx') {
            $avant = Pids 'EXCEL'
            $app = New-Object -ComObject Excel.Application
            $cree = Cree 'EXCEL' $avant
            $alertes = $app.DisplayAlerts
            $app.DisplayAlerts = $false
            try {
                $wb = $app.Workbooks.Open($f, 0, $true)
                $ws = $wb.Worksheets.Item(1)
                $u = $ws.UsedRange
                "OK  $nom (Excel) : feuilles=$($wb.Worksheets.Count) lignes=$($u.Rows.Count) colonnes=$($u.Columns.Count) A1='$($ws.Range('A1').Text)' B2='$($ws.Range('B2').Text)' gras(A1)=$($ws.Range('A1').Font.Bold)"
                $wb.Close($false)
            } finally {
                $app.DisplayAlerts = $alertes
                if ($cree) { $app.Quit() }
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
            }
        } elseif ($ext -eq '.pptx') {
            $avant = Pids 'POWERPNT'
            $app = New-Object -ComObject PowerPoint.Application
            $cree = Cree 'POWERPNT' $avant
            try {
                # Open(FileName, ReadOnly=msoTrue, Untitled=msoFalse, WithWindow=msoFalse)
                $pres = $app.Presentations.Open($f, -1, 0, 0)
                $textes = @()
                foreach ($sh in $pres.Slides.Item(1).Shapes) { if ($sh.HasTextFrame) { $textes += $sh.TextFrame.TextRange.Text -replace "[\r\n]+", ' / ' } }
                "OK  $nom (PowerPoint) : diapos=$($pres.Slides.Count) diapo1='$(($textes -join ' || ').Substring(0, [Math]::Min(110, ($textes -join ' || ').Length)))'"
                $pres.Close()
            } finally {
                if ($cree) { $app.Quit() }
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
            }
        } else {
            "??  $nom : extension non geree"
        }
    } catch {
        "ECHEC $nom : $($_.Exception.Message)"
    }
}
