#!/usr/bin/env python3
"""Entraînement du classifieur « Waly » (dev-time, WSL) — numpy pur.

Entrées : bins .f32 produits par `eveil-banc features` (lignes de 1536 =
16 pas × embedding 96-d du speech-embedding Google, gelé). Le split
train/validation se fait PAR FICHIER (= par voix) : la validation mesure la
généralisation à des timbres jamais vus à l'entraînement.

v2 (leçons du v1, FAR mots proches 30-60 %) : standardisation des features
CUITE dans l'ONNX (Sub/Div), réseau 1536→256→64→1, AdamW + cosinus,
`--neg-dur` = négatifs durs pondérés ×3 (voisins phonétiques).

Usage :
  python3 train_waly.py --pos a.f32,b.f32 --neg c.f32 --neg-dur d.f32 \
      --val-pos e.f32 --val-neg f.f32 --out waly_v2.onnx [--epochs 60]
"""

import argparse
import sys

import numpy as np

FEAT = 1536


def load_bins(paths):
    rows = []
    for p in paths:
        if not p:
            continue
        a = np.fromfile(p, dtype="<f4")
        if a.size % FEAT:
            sys.exit(f"{p}: taille non multiple de {FEAT}")
        rows.append(a.reshape(-1, FEAT))
        print(f"  {p}: {rows[-1].shape[0]} lignes")
    return np.concatenate(rows) if rows else np.zeros((0, FEAT), np.float32)


def init(shape, rng):
    return (rng.standard_normal(shape) * np.sqrt(2.0 / shape[0])).astype(np.float64)


def forward(params, x):
    w1, b1, w2, b2, w3, b3 = params
    h1 = np.maximum(x @ w1 + b1, 0)
    h2 = np.maximum(h1 @ w2 + b2, 0)
    z = h2 @ w3 + b3
    p = 1.0 / (1.0 + np.exp(-z))
    return h1, h2, p.ravel()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pos", required=True)
    ap.add_argument("--neg", required=True)
    ap.add_argument("--neg-dur", default="", help="négatifs durs, poids x3")
    ap.add_argument("--pos-fort", default="",
                    help="positifs à fort poids (x6) — l'enrollment réel")
    ap.add_argument("--val-pos", required=True)
    ap.add_argument("--val-neg", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--epochs", type=int, default=60)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--dropout", type=float, default=0.3)
    ap.add_argument("--feat-noise", type=float, default=0.15,
                    help="bruit gaussien sur les features standardisées (anti-timbre)")
    args = ap.parse_args()

    print("train:")
    xp = load_bins(args.pos.split(","))
    xpf = load_bins(args.pos_fort.split(",")) if args.pos_fort else np.zeros((0, FEAT), np.float32)
    xn = load_bins(args.neg.split(","))
    xd = load_bins(args.neg_dur.split(",")) if args.neg_dur else np.zeros((0, FEAT), np.float32)
    print("validation (voix tenues hors entraînement):")
    vp, vn = load_bins(args.val_pos.split(",")), load_bins(args.val_neg.split(","))

    x = np.concatenate([xp, xpf, xn, xd]).astype(np.float64)
    y = np.concatenate([np.ones(len(xp) + len(xpf)), np.zeros(len(xn) + len(xd))])
    # Poids : équilibre pos/neg ; l'enrollment réel compte x6, les négatifs
    # DURS (voisins + domaine réel) x3.
    w_pos = len(y) / (2.0 * max(y.sum(), 1))
    w_neg = len(y) / (2.0 * max((1 - y).sum(), 1))
    sw = np.concatenate([
        np.full(len(xp), w_pos),
        np.full(len(xpf), 6.0 * w_pos),
        np.full(len(xn), w_neg),
        np.full(len(xd), 3.0 * w_neg),
    ])

    # Standardisation (cuite dans l'ONNX à l'export).
    mu = x.mean(0)
    sd = x.std(0) + 1e-6
    x = (x - mu) / sd
    xv = (np.concatenate([vp, vn]).astype(np.float64) - mu) / sd
    yv = np.concatenate([np.ones(len(vp)), np.zeros(len(vn))])

    rng = np.random.default_rng(args.seed)
    params = [init((FEAT, 256), rng), np.zeros(256), init((256, 64), rng), np.zeros(64),
              init((64, 1), rng), np.zeros(1)]
    m = [np.zeros_like(p) for p in params]
    v = [np.zeros_like(p) for p in params]
    lr0, b1m, b2m, eps, wd, t = 2e-3, 0.9, 0.999, 1e-8, 1e-4, 0
    batch = 256
    best = (1e9, None, -1)

    for epoch in range(args.epochs):
        lr = 1e-4 + 0.5 * (lr0 - 1e-4) * (1 + np.cos(np.pi * epoch / args.epochs))
        idx = rng.permutation(len(x))
        tot = 0.0
        for i in range(0, len(x), batch):
            j = idx[i:i + batch]
            xb, yb, wb = x[j], y[j], sw[j]
            # Bruit de features : décorrèle le détail de timbre (anti-
            # mémorisation des 12 pseudo-locuteurs).
            if args.feat_noise > 0:
                xb = xb + rng.standard_normal(xb.shape) * args.feat_noise
            w1, b1, w2, b2, w3, b3 = params
            h1 = np.maximum(xb @ w1 + b1, 0)
            # Dropout inversé sur les deux couches cachées.
            if args.dropout > 0:
                m1 = (rng.random(h1.shape) > args.dropout) / (1 - args.dropout)
                h1 = h1 * m1
            h2 = np.maximum(h1 @ w2 + b2, 0)
            if args.dropout > 0:
                m2 = (rng.random(h2.shape) > args.dropout) / (1 - args.dropout)
                h2 = h2 * m2
            p = 1.0 / (1.0 + np.exp(-(h2 @ w3 + b3))).ravel()
            p = np.clip(p, 1e-7, 1 - 1e-7)
            tot += float(np.sum(wb * -(yb * np.log(p) + (1 - yb) * np.log(1 - p))))
            dz3 = (wb * (p - yb))[:, None] / len(xb)
            g = [None] * 6
            g[4] = h2.T @ dz3
            g[5] = dz3.sum(0)
            dh2 = dz3 @ params[4].T
            if args.dropout > 0:
                dh2 = dh2 * m2
            dz2 = dh2 * (h2 > 0)
            g[2] = h1.T @ dz2
            g[3] = dz2.sum(0)
            dh1 = dz2 @ params[2].T
            if args.dropout > 0:
                dh1 = dh1 * m1
            dz1 = dh1 * (h1 > 0)
            g[0] = xb.T @ dz1
            g[1] = dz1.sum(0)
            t += 1
            for k in range(6):
                m[k] = b1m * m[k] + (1 - b1m) * g[k]
                v[k] = b2m * v[k] + (1 - b2m) * g[k] ** 2
                mh = m[k] / (1 - b1m ** t)
                vh = v[k] / (1 - b2m ** t)
                params[k] -= lr * mh / (np.sqrt(vh) + eps)
                if k % 2 == 0:  # decay sur les poids, pas les biais
                    params[k] -= lr * wd * params[k]
        _, _, pv = forward(params, xv)
        pv = np.clip(pv, 1e-7, 1 - 1e-7)
        vloss = float(np.mean(-(yv * np.log(pv) + (1 - yv) * np.log(1 - pv))))
        frr = float(np.mean(pv[yv == 1] < 0.5)) * 100
        far = float(np.mean(pv[yv == 0] >= 0.5)) * 100
        # Sélection sur la MÉTRIQUE DE DÉCISION au seuil d'usage (0,8),
        # pas la val loss (leçon v3 : elle choisissait un modèle sous-appris,
        # 30 % de FRR séquentiel même sur voix d'entraînement).
        frr8 = float(np.mean(pv[yv == 1] < 0.8)) * 100
        far8 = float(np.mean(pv[yv == 0] >= 0.8)) * 100
        metric = frr8 + 2.0 * far8
        print(f"époque {epoch:2d}  lr {lr:.2e}  train {tot / len(x):.4f}  val {vloss:.4f}  "
              f"FRR@0,5 {frr:5.2f} %  FAR@0,5 {far:6.3f} %  "
              f"[décision@0,8 : FRR {frr8:5.2f} + 2×FAR {far8:6.3f} = {metric:.2f}]")
        if metric < best[0]:
            best = (metric, [p.copy() for p in params], epoch)

    params = best[1]
    export_onnx(params, mu, sd, args.out)
    print(f"exporté : {args.out} (métrique décision {best[0]:.2f} à l'époque {best[2]})")


def export_onnx(params, mu, sd, path):
    import onnx
    from onnx import TensorProto, helper, numpy_helper

    w1, b1, w2, b2, w3, b3 = [p.astype(np.float32) for p in params]
    inits = [
        numpy_helper.from_array(np.array([1, FEAT], np.int64), "shape_flat"),
        numpy_helper.from_array(mu.astype(np.float32), "mu"),
        numpy_helper.from_array(sd.astype(np.float32), "sd"),
        numpy_helper.from_array(w1, "w1"), numpy_helper.from_array(b1, "b1"),
        numpy_helper.from_array(w2, "w2"), numpy_helper.from_array(b2, "b2"),
        numpy_helper.from_array(w3, "w3"), numpy_helper.from_array(b3, "b3"),
    ]
    nodes = [
        helper.make_node("Reshape", ["x", "shape_flat"], ["flat"]),
        helper.make_node("Sub", ["flat", "mu"], ["cent"]),
        helper.make_node("Div", ["cent", "sd"], ["std"]),
        helper.make_node("Gemm", ["std", "w1", "b1"], ["z1"]),
        helper.make_node("Relu", ["z1"], ["h1"]),
        helper.make_node("Gemm", ["h1", "w2", "b2"], ["z2"]),
        helper.make_node("Relu", ["z2"], ["h2"]),
        helper.make_node("Gemm", ["h2", "w3", "b3"], ["z3"]),
        helper.make_node("Sigmoid", ["z3"], ["y"]),
    ]
    graph = helper.make_graph(
        nodes, "waly_wake",
        [helper.make_tensor_value_info("x", TensorProto.FLOAT, [1, 16, 96])],
        [helper.make_tensor_value_info("y", TensorProto.FLOAT, [1, 1])],
        inits,
    )
    model = helper.make_model(
        graph, opset_imports=[helper.make_opsetid("", 13)], ir_version=8
    )
    onnx.checker.check_model(model)
    onnx.save(model, path)


if __name__ == "__main__":
    main()
