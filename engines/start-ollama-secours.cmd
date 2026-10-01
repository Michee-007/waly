@echo off
rem Secours LLM : Ollama (signe, passe SAC) sur le port de l'app.
rem Port 42626 = HORS de la zone dynamique Windows (49152-65535) : WinNAT
rem ne peut pas le reserver (vecu 2026-09-10 : 52626 reserve -> bind 10013).
rem Surcharge : WALY_LLM_PORT (le meme que l'app).
if "%WALY_LLM_PORT%"=="" set WALY_LLM_PORT=42626
set OLLAMA_HOST=127.0.0.1:%WALY_LLM_PORT%
"%LOCALAPPDATA%\Programs\Ollama\ollama.exe" serve > "%~dp0ollama-secours.log" 2>&1
