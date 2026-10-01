# Cible du banc B (GATE 2/3) : une fenetre WinForms A NOUS, pour agir sans
# jamais toucher aux fenetres de l'utilisateur. ASCII pur (piege 2).
# Lancer : powershell -ExecutionPolicy Bypass -File cible-banc.ps1
Add-Type -AssemblyName System.Windows.Forms
$f = New-Object System.Windows.Forms.Form
$f.Text = "Cible banc Waly"
$f.Width = 420; $f.Height = 300; $f.StartPosition = "CenterScreen"
# Reduite au lancement : ne vole ni le focus ni l'ecran de l'utilisateur
# (et prouve l'action en ARRIERE-PLAN). -Visible pour l'afficher normalement.
if ($args -notcontains "-Visible") { $f.WindowState = "Minimized" }

$lbl = New-Object System.Windows.Forms.Label
$lbl.Text = "Nom du fichier"; $lbl.Left = 20; $lbl.Top = 20; $lbl.Width = 120
$f.Controls.Add($lbl)

$nom = New-Object System.Windows.Forms.TextBox
$nom.Name = "nom"; $nom.AccessibleName = "Nom du fichier"
$nom.Left = 150; $nom.Top = 18; $nom.Width = 220
$f.Controls.Add($nom)

$mdp = New-Object System.Windows.Forms.TextBox
$mdp.AccessibleName = "Mot de passe"; $mdp.UseSystemPasswordChar = $true
$mdp.Left = 150; $mdp.Top = 50; $mdp.Width = 220
$f.Controls.Add($mdp)

$chk = New-Object System.Windows.Forms.CheckBox
$chk.Text = "Ajouter la date"; $chk.Left = 20; $chk.Top = 85; $chk.Width = 200
$f.Controls.Add($chk)

$cmb = New-Object System.Windows.Forms.ComboBox
$cmb.AccessibleName = "Format"; $cmb.Left = 150; $cmb.Top = 115; $cmb.Width = 120
[void]$cmb.Items.AddRange(@("Texte", "Markdown", "PDF"))
$f.Controls.Add($cmb)

$etat = New-Object System.Windows.Forms.Label
$etat.Text = "Pret"; $etat.Left = 20; $etat.Top = 200; $etat.Width = 360
$f.Controls.Add($etat)

$btn = New-Object System.Windows.Forms.Button
$btn.Text = "Enregistrer"; $btn.Left = 20; $btn.Top = 150; $btn.Width = 120
$btn.Add_Click({ $etat.Text = "Enregistre : " + $nom.Text + " (date=" + $chk.Checked + ", format=" + $cmb.Text + ")" })
$f.Controls.Add($btn)

[void]$f.ShowDialog()
