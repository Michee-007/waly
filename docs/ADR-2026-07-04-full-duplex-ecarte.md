# ADR — Modèles vocaux full-duplex écartés (Moshi/Moshika, PersonaPlex)

Date : 2026-07-04 · Statut : accepté · Contexte : R1.5 (voix temps réel)

## Question
Pourquoi une cascade VAD→STT→LLM→TTS plutôt qu'un modèle parole-à-parole
full-duplex comme `kyutai/moshika-rl-seamless` (Moshi 7,7 Md) ou
`kyutai/personaplex-rl-seamless` (NVIDIA PersonaPlex 7 Md), qui offrent
nativement tour de parole, backchannel et barge-in (RL sur Seamless
Interaction, 4 000 h) ?

## Décision
Cascade maintenue. Full-duplex bout-en-bout écarté sur cette machine et
cette feuille de route.

## Raisons, par ordre de dureté
1. **Mur matériel.** Décodage borné bande passante : 16-19 tok/s pour un 4B
   (mesuré R0) ; l'audit a écarté définitivement la classe 8B. Un full-duplex
   7 Md exige une génération CONTINUE à 12,5 trames/s sans à-coups — plus dur
   qu'un chat texte. Budget IA ~6,5 Go : le modèle prendrait tout (rien pour
   mémoire/outils/vision).
2. **Cerveau prisonnier.** Dans un parole-à-parole, le LLM est le modèle
   vocal : pas de cerveau interchangeable, raisonnement/outils plus faibles
   qu'un LLM texte à taille égale. R2+ (mémoire, outils, dispatch supervisé)
   suppose un cerveau texte piloté. La cascade rend la voix périphérique
   (preuve : Piper→Pocket sans toucher au reste).
3. **Pas de chemin NPU.** Modèles PyTorch, aucun runtime NPU ; notre atout
   latence est le préfill FLM NPU (×16).
4. **Français.** Moshika/PersonaPlex : anglais d'abord ; Waly est FR.

## Ce qu'on leur emprunte quand même
Leur RL d'interactivité définit LA cible UX de R1.5 : fin de tour
sémantique, barge-in bien placé, backchannel. Notre version cascade :
endpointing spéculatif Parakeet + barge-in Silero dédié (validés terrain
2026-07-04).

## Conditions de réouverture
Machine ≥ 32 Go avec décodage 7B temps réel (NPU ou GPU), OU distillation
full-duplex < 2 Md FR de qualité. Réévaluer alors le front vocal seulement —
le cerveau texte et les outils restent.
