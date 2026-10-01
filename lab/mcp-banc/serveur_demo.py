"""Serveur MCP de banc (stdio, bibliotheque standard seule).

Prouve le client MCP de Waly de bout en bout sans rien telecharger :
JSON-RPC 2.0 ligne a ligne sur stdin/stdout, poignee de main, tools/list,
tools/call. Deux outils purement locaux : compter_mots, inverser.

Lancement direct (debug) : python serveur_demo.py  puis coller du JSON.
"""
import json
import sys

sys.stdin.reconfigure(encoding="utf-8")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")

OUTILS = [
    {
        "name": "compter_mots",
        "description": "Compte les mots d'un texte (outil de banc MCP).",
        "inputSchema": {
            "type": "object",
            "properties": {"texte": {"type": "string", "description": "le texte"}},
            "required": ["texte"],
        },
    },
    {
        "name": "inverser",
        "description": "Renvoie le texte a l'envers (outil de banc MCP).",
        "inputSchema": {
            "type": "object",
            "properties": {"texte": {"type": "string"}},
            "required": ["texte"],
        },
    },
]


def repondre(id_, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": id_}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    sys.stdout.write(json.dumps(msg, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def texte(t, erreur=False):
    return {"content": [{"type": "text", "text": t}], "isError": erreur}


for ligne in sys.stdin:
    ligne = ligne.strip()
    if not ligne:
        continue
    try:
        m = json.loads(ligne)
    except ValueError:
        continue
    methode, id_ = m.get("method"), m.get("id")
    if id_ is None:
        continue  # notification (notifications/initialized...)
    if methode == "initialize":
        repondre(id_, {
            "protocolVersion": m.get("params", {}).get("protocolVersion", "2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "waly-banc-demo", "version": "0.1"},
        })
    elif methode == "tools/list":
        repondre(id_, {"tools": OUTILS})
    elif methode == "tools/call":
        p = m.get("params", {})
        nom, args = p.get("name"), p.get("arguments") or {}
        t = str(args.get("texte", ""))
        if nom == "compter_mots":
            repondre(id_, texte(f"{len(t.split())} mots"))
        elif nom == "inverser":
            repondre(id_, texte(t[::-1]))
        else:
            repondre(id_, texte(f"outil inconnu: {nom}", erreur=True))
    elif methode == "ping":
        repondre(id_, {})
    else:
        repondre(id_, error={"code": -32601, "message": f"methode inconnue: {methode}"})
