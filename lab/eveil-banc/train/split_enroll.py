#!/usr/bin/env python3
"""Découpe l'enregistrement continu d'enrollment en prises « Waly ».

Segmentation par énergie : RMS sur fenêtres de 30 ms, seuil relatif au
plancher de bruit, fusion des rafales proches (< 300 ms), prises de
0,15-2,5 s exportées avec 80 ms de marge. Écrit aussi le split des
négatifs (240 s train / 60 s validation).
"""

import sys
import wave
from pathlib import Path

import numpy as np

RATE = 16000


def read(path):
    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768.0


def write(path, x):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype("<i2").tobytes())


def main():
    raw_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("corpus/enroll/michee-raw")
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("corpus/enroll/michee")
    out_dir.mkdir(parents=True, exist_ok=True)

    x = read(raw_dir / "positifs-brut.wav")
    win = RATE * 30 // 1000
    n_win = len(x) // win
    rms = np.sqrt((x[: n_win * win].reshape(-1, win) ** 2).mean(1))
    # Seuil ABSOLU d'abord : une pièce calme a un plancher ~0 (vécu : le
    # relatif au plancher ratait les prises douces/à 3 m).
    th = max(0.008, rms.max() * 0.04)
    on = rms > th

    # Fusion des trous < 300 ms, extraction des rafales.
    segs = []
    i = 0
    while i < len(on):
        if on[i]:
            j = i
            gap = 0
            while j < len(on) and gap <= 10:  # 10 fenêtres = 300 ms
                gap = gap + 1 if not on[j] else 0
                j += 1
            segs.append((i, j - gap))
            i = j
        else:
            i += 1

    margin = RATE * 80 // 1000
    kept = 0
    durs = []
    for a, b in segs:
        s = max(a * win - margin, 0)
        e = min(b * win + margin, len(x))
        dur = (e - s) / RATE
        if 0.15 <= dur <= 2.5:
            kept += 1
            durs.append(round(dur, 2))
            write(out_dir / f"waly_{kept:03d}.wav", x[s:e])
    print(f"{kept} prises gardées (sur {len(segs)} rafales) ; durées {durs}")

    n = read(raw_dir / "negatifs-brut.wav")
    cut = RATE * 240
    neg_tr = raw_dir.parent / "michee-neg-train"
    neg_va = raw_dir.parent / "michee-neg-val"
    neg_tr.mkdir(exist_ok=True)
    neg_va.mkdir(exist_ok=True)
    write(neg_tr / "neg-train.wav", n[:cut])
    write(neg_va / "neg-val.wav", n[cut:])
    print(f"négatifs : {cut / RATE:.0f} s train, {(len(n) - cut) / RATE:.0f} s val")


if __name__ == "__main__":
    main()
