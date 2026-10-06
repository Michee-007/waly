; Installeur Waly (R3 ch. 4) — NSIS 3, compile depuis WSL avec le makensis
; Ubuntu extrait sans root (voir apps/desktop/installer/README.md).
; Choix : installation PER-USER ($LOCALAPPDATA\Programs\Waly, zero UAC),
; raccourcis Menu Demarrer + Bureau, desinstalleur enregistre dans HKCU.
; La desinstallation NE TOUCHE PAS aux donnees (C:\waly\data) : la memoire
; de Waly appartient a l'utilisateur, pas a l'installeur.
; Prerequis machine : WebView2 Runtime (present sur tout Windows 11 a jour).
; ASCII pur (culture piege 2) ; textes sans accents assumes.

; Sel Smart App Control (piege 3) : le verdict vaut par binaire. Si
; l'installeur est bloque, relancer build-installer.sh avec SAC_SEL=n : le
; sel entre dans les informations de version, donc dans l'empreinte.
!ifndef SAC_SEL
  !define SAC_SEL 1
!endif
Unicode true
Name "Waly"
VIProductVersion "0.1.0.${SAC_SEL}"
VIAddVersionKey "ProductName" "Waly"
VIAddVersionKey "FileDescription" "Installeur de Waly"
VIAddVersionKey "FileVersion" "0.1.0.${SAC_SEL}"
VIAddVersionKey "LegalCopyright" "MIT"
OutFile "${OUT_PATH}"
InstallDir "$LOCALAPPDATA\Programs\Waly"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "${ICON_PATH}"
UninstallIcon "${ICON_PATH}"

; Pas d'assistant : double-clic = installation directe puis lancement.
Page instfiles

Section "Waly"
  SetOutPath "$INSTDIR"
  File /oname=Waly.exe "${EXE_PATH}"
  File "${LOADER_PATH}"
  ; Huis clos (ADR 2026-07-21) : le service scelleur est BUNDLE. On le pose en
  ; staging dans INSTDIR puis on lance `setup` ELEVE (une seule UAC) : il se
  ; recopie dans Program Files (hors ecriture utilisateur = pas d'escalade),
  ; s'enregistre en AUTO_START et demarre. Scelle par defaut, des le boot.
  File "${SVC_PATH}"
  WriteUninstaller "$INSTDIR\Desinstaller.exe"

  ; Enregistrement du service (elevation : Program Files + SCM). ExecShellWait
  ; attend la fin ; le code de sortie n'est pas bloquant (l'app affichera
  ; "reseau non scelle - reparer" si le service manque - honnetete du degrade).
  ExecShellWait "runas" "$INSTDIR\waly-seal-svc.exe" "setup" SW_HIDE

  CreateDirectory "$SMPROGRAMS\Waly"
  CreateShortcut "$SMPROGRAMS\Waly\Waly.lnk" "$INSTDIR\Waly.exe"
  CreateShortcut "$DESKTOP\Waly.lnk" "$INSTDIR\Waly.exe"

  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "DisplayName" "Waly - assistant personnel local"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "UninstallString" "$\"$INSTDIR\Desinstaller.exe$\""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "DisplayIcon" "$\"$INSTDIR\Waly.exe$\""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "Publisher" "Waly (local)"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "DisplayVersion" "0.1.0"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly" \
    "NoRepair" 1

  ; Double-clic installe ET lance (critere de sortie R3).
  Exec "$INSTDIR\Waly.exe"
SectionEnd

Section "Uninstall"
  ; Retirer le service (eleve : arret + suppression SCM + nettoyage de la copie
  ; Program Files). Une UAC a la desinstallation.
  ExecShellWait "runas" "$INSTDIR\waly-seal-svc.exe" "uninstall" SW_HIDE
  Delete "$INSTDIR\waly-seal-svc.exe"
  Delete "$INSTDIR\Waly.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\Desinstaller.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Waly\Waly.lnk"
  RMDir "$SMPROGRAMS\Waly"
  Delete "$DESKTOP\Waly.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Waly"
  ; C:\waly\data (memoire, conversations) volontairement conserve.
SectionEnd
