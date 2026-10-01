# waly-relais

Le relais qui permet à deux Waly de s'envoyer une conversation, où qu'ils
soient dans le monde, **sans compte et sans pouvoir la lire**.

Il garde des enveloppes chiffrées dans des boîtes, le temps que leur
destinataire les relève. C'est tout : pas de base de données, pas de fichier,
pas de compte, pas de journal de contenu ni d'adresses.

## Ce que le relais voit — et ne voit pas

| Il voit | Il ne voit pas |
|---|---|
| qu'une enveloppe arrive dans telle boîte, sa taille, l'heure | le contenu (chiffré de bout en bout, `crypto_box` de NaCl) |
| l'adresse IP de qui dépose et de qui relève (comme tout serveur) | qui est derrière une boîte (128 bits au hasard, aucun nom) |
| la clé publique de l'expéditeur, écrite sur l'enveloppe | la clé secrète de quiconque |

Un relais malveillant peut refuser de livrer ou effacer des enveloppes. Il ne
peut ni les lire, ni les modifier sans que le destinataire le détecte, ni en
fabriquer au nom d'un contact.

## L'héberger

Un petit serveur Linux suffit (le programme est en Rust pur, bibliothèque
standard).

```bash
cargo build --release -p waly-relais
./target/release/waly-relais 127.0.0.1:8787
```

Le relais écoute en clair sur la boucle locale : on met un mandataire TLS
devant. Avec Caddy (certificat automatique) :

```
relais.exemple.fr {
    reverse_proxy 127.0.0.1:8787
}
```

Dans Waly : Paramètres › Partage et contacts › adresse du relais
`https://relais.exemple.fr`. Waly refuse une adresse `http://` qui ne désigne
pas la machine elle-même ou le réseau local.

Un seul relais sert autant de personnes qu'on veut : chacun y a sa boîte.
Deux contacts peuvent aussi avoir chacun le leur — on dépose toujours sur le
relais du DESTINATAIRE (son adresse est dans son code Waly).

## Protocole

| Requête | Effet |
|---|---|
| `GET /v1/boites/<boîte>` + `Authorization: Bearer <jeton>` (+ `X-Attente: s`) | relève ; la première relève ouvre la boîte et lui attache ce jeton |
| `POST /v1/boites/<boîte>` | dépôt d'une enveloppe dans une boîte ouverte → 201 |
| `DELETE /v1/boites/<boîte>/<id>` + jeton | l'enveloppe relevée quitte le relais |
| `GET /v1/sante` | `"ok"` |

`<boîte>` = 32 caractères hexadécimaux, `<jeton>` = 64.

## Bornes

Enveloppe ≤ 1 Mo · 100 enveloppes par boîte · garde 7 jours · boîte fermée
après 30 jours sans relève · 5 000 boîtes · tout en mémoire : un redémarrage
vide le relais (l'expéditeur renvoie).

## Limites connues

- Pas de limitation de débit par adresse : à mettre dans le mandataire si le
  relais est public.
- Pas de confidentialité persistante : une clé secrète volée plus tard ouvre
  les enveloppes encore gardées au relais (7 jours au plus).
