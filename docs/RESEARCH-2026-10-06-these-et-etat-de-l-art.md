# Recherche 2026-10-06 — la thèse de Waly face à la recherche et à l'état de l'art

> Deux questions, posées avant de parler du projet en public :
> 1. La thèse de Waly tient-elle devant ce que la recherche a établi
>    (économie, sécurité, fiabilité des agents) ?
> 2. Ce que nous affirmons est-il mieux fait ailleurs ?
>
> Règle : chaque source est marquée **lue** (résumé ou page d'origine ouverts
> le 2026-10-06) ou **secondaire** (rapportée par un tiers, à relire avant de
> la citer). Rien n'est cité de mémoire sans le dire.

## 1. La thèse, en une phrase

**Un système d'intelligence personnelle ne vaut que ce que son propriétaire
peut vérifier à bas coût.** Il relie toute une vie numérique et agit pour une
seule personne ; sa valeur n'est donc pas plafonnée par son intelligence, mais
par la confiance qu'on peut lui accorder sans passer sa vie à le surveiller.
Waly prend ce plafond pour objet : ce qui sort, ce qui est fait et ce qui est
su doit pouvoir se contrôler d'un geste.

## 2. Ce que la recherche établit

| Source | Statut | Ce qu'elle établit | Ce que ça implique ici |
|---|---|---|---|
| Shahidi, Rusak, Manning, Fradkin, Horton, *The Coasean Singularity? Demand, Supply, and Market Design with AI Agents*, NBER w34468, nov. 2025 | lue | La demande d'agents est une demande dérivée : l'utilisateur arbitre entre qualité de décision et effort épargné. | Un agent personnel se juge à l'effort qu'il retire **net** de l'effort de contrôle qu'il ajoute. |
| Catalini, Hui, Wu, *Some Simple Economics of AGI*, arXiv 2602.20946, fév. 2026 | lue | Le coût d'exécuter tombe, le coût de **vérifier** reste borné par l'humain. L'écart plafonne la valeur réalisée. Le défi : « sécuriser les fondations de la supervision », pas déployer plus d'autonomie. | La vérification est la ressource rare. La rendre bon marché est un produit, pas une précaution. |
| *The Human-AI Delegation-Verification Dilemma*, arXiv 2605.21351, 2026 | lue | Chacun a intérêt à déléguer sans vérifier ; agrégé, c'est un dilemme du prisonnier qui dégrade les standards communs. La sortie passe par des engagements. | La vérification ne doit pas dépendre de la bonne volonté : état sûr par défaut, preuve sans effort. |
| Rabanser, Kapoor, Kirgis, Liu, Utpala, Narayanan, *Towards a Science of AI Agent Reliability*, ICML 2026, arXiv 2602.16666 | lue | Sur 15 modèles, les gains de capacité ne se traduisent que modestement en fiabilité. | Un agent plus fort n'est pas un agent plus sûr : la supervision reste nécessaire. |
| Kalai, Nachum, Vempala, Zhang, *Why Language Models Hallucinate*, arXiv 2509.04664, 2025 | lue | Les modèles devinent parce que l'évaluation récompense la réponse plutôt que l'aveu d'incertitude. | Dire « je ne vois pas » est un choix de conception à défendre. |
| Willison, *The lethal trifecta for AI agents*, 16 juin 2025 | lue | Données privées + contenu non maîtrisé + communication vers l'extérieur = exfiltration possible. | Un assistant personnel réunit les deux premiers par définition. |
| Meta, *Agents Rule of Two*, 31 oct. 2025 | lue | Un agent ne doit réunir que deux des trois propriétés dans une session ; s'il lui faut les trois, il ne doit pas agir seul : approbation humaine. | C'est exactement le contrat de Waly : la troisième propriété n'existe que par une porte ouverte à la main. |
| Microsoft Research, *Magentic Marketplace*, arXiv 2510.25779, 2025 | lue | **Qwen3-4B-2507** — le cerveau de secours de Waly — cède aux injections de prompt et aux manipulations, là où les grands modèles tiennent. | Sur un petit modèle local, la garantie **ne peut pas** vivre dans le modèle. |
| Bagdasarian et al., *AirGapAgent*, CCS 2024, arXiv 2405.05175 | lue | Ne donner à un tiers que les données nécessaires à la tâche fait passer la protection de 45 % à 97 % sous attaque. | C'est le principe de `conversation_sortante` : un modèle extérieur ne reçoit que le texte affiché. |
| Chung, Badhe, *Local Is Not a Sufficient Privacy Boundary*, arXiv 2606.10173, juin 2026 | lue | « Local » dit où se fait le calcul, pas ce qui sort ni qui a autorité. Il faut des contrôles visibles et auditables. | « 100 % local » n'est pas un argument en soi. |
| Erickson, *Who Does Your AI Work For?*, CUI 2026, arXiv 2605.28908 | lue | Un agent conversationnel devrait un devoir de loyauté à son utilisateur, comme un professionnel. | Un seul mandant : pas de compte, pas de télémétrie, pas d'intérêt tiers. |
| Kolt, *Governing AI Agents*, arXiv 2501.07913 | secondaire | Le problème principal-agent s'applique aux agents IA : asymétrie d'information, autorité, loyauté. | Même cadre que la finance : déléguer exige de pouvoir contrôler. |
| Hammond et al., *Multi-Agent Risks from Advanced AI*, arXiv 2502.14143 ; Cemri et al., *Why Do Multi-Agent LLM Systems Fail?*, arXiv 2503.13657 | secondaire | Les systèmes à plusieurs agents échouent d'abord par la coordination et l'absence de vérification, pas par le modèle. | À garder en tête pour les missions et pour le futur appel entre deux Waly. |
| Stivers et al., PNAS 2009 ; Levinson et Torreira, 2015 | lue | Dix langues : on se répond en ~200 ms, alors que préparer une phrase en demande 600 ou plus. On anticipe la fin du tour. | La fin de tour compte plus que le débit du modèle. |
| *International AI Safety Report 2026* | secondaire | Les agents progressent vite mais ne sont pas fiables pour des opérations longues sans supervision. | Cohérent avec le reste ; à relire avant de citer un chiffre. |

Pont avec la finance (cité de mémoire, **à relire**) : la « vérification
coûteuse de l'état » de Townsend (1979) explique la forme des contrats de
dette par le coût d'audit. Même mécanique ici : quand vérifier coûte cher, on
délègue moins ; faire baisser ce coût élargit ce qu'on peut confier.

## 3. La thèse, en cinq propositions

1. **Un système d'intelligence personnelle réunit par définition deux des
   trois ingrédients du risque** (données privées, contenu non maîtrisé). La
   communication vers l'extérieur doit donc être l'exception, ouverte par un
   humain. *Dans Waly :* scellé par défaut, sorties ouvertes une par une.
2. **La garantie ne peut pas vivre dans le modèle.** Un petit modèle local
   est mesuré comme le plus manipulable. *Dans Waly :* ce qui sort est décidé
   par le harnais (ce qu'une porte transporte) et par le noyau (pas de porte
   par défaut), jamais par le bon vouloir du modèle.
3. **La valeur est plafonnée par le coût de vérification.** *Dans Waly :* un
   essai réel et un journal, dans l'interface. Depuis le 2026-10-06, l'essai
   est fait d'office : l'état affiché est prouvé, plus seulement déclaré.
4. **Personne ne vérifie spontanément.** *Dans Waly :* l'état sûr est l'état
   par défaut, il n'y a rien à activer.
5. **Un seul mandant.** *Dans Waly :* aucun compte, aucune télémétrie, la
   mémoire ne quitte pas la machine.

Ce que la thèse ne dit pas : que Waly est plus intelligent, plus rapide ou
plus complet que les agents du marché. Il ne l'est pas.

## 4. Où c'est mieux fait ailleurs

| Sujet | Ce que fait Waly | Ce qui existe | Écart honnête |
|---|---|---|---|
| Scellé réseau sous Windows | Filtres WFP **par chemin d'exécutable**, posés par un service SYSTEM, fail-closed, journal. | **Anthropic `sandbox-runtime`** (lue ; Windows en alpha) : filtres WFP **par identité de compte dédié**, plus un proxy local qui n'autorise que les domaines déclarés. | **Leur conception couvre les programmes enfants et filtre par domaine ; la nôtre non.** Un programme lancé par Waly sous un autre nom d'exécutable n'est pas scellé. Problème ouvert n° 1. |
| Bac à sable d'agent sous Windows | — | **OpenAI Codex** (lue) : comptes dédiés et règles de pare-feu, installation administrateur. Pour un agent de code. | Même famille de solution que la nôtre, appliquée à un autre objet. |
| Confinement d'agents personnels | — | **NVIDIA NemoClaw / OpenShell** (lue ; alpha) : seules les destinations approuvées sortent. Linux, macOS, WSL 2 expérimental. | Pas de Windows natif. |
| Confinement par le système | — | **Windows 11, comptes d'agent et espace de travail d'agent** (lue ; expérimental, désactivé par défaut). Conteneurs d'exécution annoncés à Build 2026 (secondaire). | Si Microsoft le livre, le système fera une partie de ce que fait `waly-seal`. À suivre de près. |
| Blocage par application | Même mécanisme WFP. | **simplewall, Portmaster, Fort Firewall** (secondaire) : pare-feux libres, plus généraux, avec journaux. | Bloquer un programme n'est pas neuf. Notre apport est étroit : état par défaut, service à protocole étroit, essai et journal intégrés à l'assistant. |
| Fin de tour de parole | Heuristique sur la transcription (`endpoint.rs`). | **Pipecat Smart Turn v3** (secondaire) : modèle audio libre de 8 Mo, en ONNX. | Probablement meilleur que notre heuristique, et compatible avec notre chaîne ONNX. À mesurer. |
| Agent d'écran | Lecture en texte par UI Automation. | **Microsoft UFO²/UFO³** (secondaire) : UI Automation **et** vision, repli sur l'image quand l'arbre est pauvre. | Leur approche hybride couvre les applications sans arbre d'accessibilité. |
| Agent personnel complet | Voix, vision, mémoire, missions. | **OpenClaw** (lue : 391 000 étoiles), **Hermes** (secondaire) : bien plus de connecteurs, de plateformes et d'utilisateurs ; Hermes a la voix, le mot d'éveil et un pare-feu d'identifiants. | Ils sont devant en étendue. |
| Preuve côté nuage | — | **Apple Private Cloud Compute** (secondaire) : attestation vérifiable par des tiers. | Autre voie vers la même exigence : prouver plutôt que promettre. |

Ce que nous n'avons trouvé nulle part réuni, à la date de cette recherche :
un assistant à voix et à vision, pour un particulier, scellé par défaut, avec
l'essai et le journal dans l'interface. C'est une combinaison, pas une
invention : chaque brique existe ailleurs, parfois en mieux.

## 5. Ce que cette recherche a changé dans le dépôt

- **La phrase « No other assistant we know of does this » est retirée** du
  README : elle était fausse au vu du tableau ci-dessus.
- **L'état du scellé est désormais prouvé par un essai** (`sceau::sonder`),
  et le service refuse de répondre « scellé » pour un programme qu'il ne voit
  pas. Voir `AUDIT-2026-10-02-promesses-rejouees.md`, défaut n° 6.
- **Les limites deviennent des problèmes ouverts**, avec la conception connue
  qui les résout (même document).

## 6. Sources

Lues le 2026-10-06 :
arxiv.org/abs/2602.20946 · arxiv.org/abs/2605.21351 ·
arxiv.org/abs/2602.16666 · arxiv.org/abs/2509.04664 ·
arxiv.org/abs/2510.25779 · arxiv.org/abs/2405.05175 ·
arxiv.org/abs/2606.10173 · arxiv.org/abs/2605.28908 ·
nber.org/papers/w34468 · simonwillison.net/2025/Jun/16/the-lethal-trifecta ·
ai.meta.com/blog/practical-ai-agent-security ·
github.com/anthropics/sandbox-runtime (README) ·
learn.chatgpt.com/docs/windows/windows-sandbox ·
github.com/NVIDIA/NemoClaw · docs.nvidia.com/openshell ·
learn.microsoft.com/windows/security/book/operating-system-agentic-security ·
github.com/openclaw/openclaw · pmc.ncbi.nlm.nih.gov/articles/PMC2705608 ·
frontiersin.org/articles/10.3389/fpsyg.2015.00731

Secondaires, à relire avant citation : arxiv.org/abs/2501.07913 ·
arxiv.org/abs/2502.14143 · arxiv.org/abs/2503.13657 ·
internationalaisafetyreport.org · huggingface.co/pipecat-ai/smart-turn-v3 ·
microsoft.github.io/UFO · hermes-agent.nousresearch.com/docs.
