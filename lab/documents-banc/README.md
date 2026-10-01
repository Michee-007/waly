# Banc « documents » — preuve par les vrais lecteurs

Les documents que Waly génère (`crates/waly-core/src/documents/` : .docx,
.xlsx, .pptx, .pdf depuis le markdown) ne se jugent pas sur des tests
unitaires seuls : **on les fait ouvrir par les vrais lecteurs**.

## 1. Générer les exemples

Depuis WSL (dossier de sortie au choix) :

```bash
WALY_DOC_OUT=/mnt/c/chemin/sortie cargo test -p waly-core ecrire_exemples -- --ignored
```

Écrit `exemple.docx`, `exemple.xlsx`, `exemple.pptx`, `exemple.pdf` depuis
le markdown `EXEMPLE` de `documents/mod.rs`.

## 2. Ouvrir dans Office (Word, Excel, PowerPoint)

```powershell
.\verif-office.ps1 -Fichiers C:\chemin\sortie\exemple.docx, C:\chemin\sortie\exemple.xlsx, C:\chemin\sortie\exemple.pptx
```

Lecture seule, fenêtre invisible, hors fichiers récents. Le script **ne
quitte une application que s'il l'a lancée** : les documents ouverts par
l'utilisateur restent intacts.

## 3. Le PDF : moteur PDF de Windows + rendu à regarder

```powershell
.\verif-pdf.ps1 -Pdf C:\chemin\sortie\exemple.pdf -Png C:\chemin\sortie\page1.png
```

`Windows.Data.Pdf` (WinRT) charge le fichier, compte les pages et rend la
page 1 en PNG : aucune application ouverte, aucun dialogue possible.

⚠ **Jamais de PDF dans Word par COM** : sa boîte « conversion de PDF » est
invisible en automatisation et bloque le script indéfiniment (vécu
2026-09-10).

## Verdict du 2026-09-10

| Format | Lecteur | Résultat |
|---|---|---|
| .docx | Word 16 | 23 paragraphes, 1 tableau, 2 listes, style `Heading 1` |
| .xlsx | Excel 16 | 1 feuille, 14 lignes, 3 colonnes, A1 en gras |
| .pptx | PowerPoint 16 | 4 diapos, titre + texte |
| .pdf | Windows.Data.Pdf | 2 pages, rendu propre (accents, gras, italique, puces, tableau) |
