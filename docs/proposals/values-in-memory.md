# Proposition : les grosses valeurs en mémoire, et mises à jour sur place

Statut : **demandée par l'auteur le 2026-11-16** (« je ne veux pas battre ce programme spécifiquement, mais battre Rust dans tous les cas, quitte à modifier le compilateur en profondeur »), après la comparaison de `bench/rtiow`. Premier chantier d'une série : chaque cause d'écart avec Rust se corrige dans le compilateur ou le runtime, jamais dans un programme, et se mesure sur l'ensemble des programmes comparés.

## 1. Le constat

Sur rtiow, Isocron est 2,5 fois plus lent que Rust sur un fil : 3,4 fois plus d'instructions, 4,2 fois plus de lectures, 7 fois plus d'écritures en mémoire, au même nombre d'instructions par cycle. Le même écart se reproduit hors du traceur, sur un cas minimal général (`bench/records/walk`) : un arbre de 4096 intervalles parcouru pour un million de requêtes, une fonction récursive qui reçoit un état de 13 mots, le rend avancé, et rend le point le plus proche (15 mots). C'est la forme du parcours d'un traceur, d'un interprète qui porte sa machine, d'un analyseur qui porte sa position.

Même résultat partout (607221459661), un cœur fixe à 3,2 GHz :

| Version | Temps | Instructions | Lectures | Écritures |
| --- | ---: | ---: | ---: | ---: |
| Rust, l'état prêté (`&mut State`) | 0,88 s | 5,5 G | 1,4 G | 1,1 G |
| Rust dans le style d'Isocron (l'état donné et rendu) | 1,18 s | 8,7 G | 2,9 G | 2,7 G |
| Isocron | 2,02 s | 16,4 G | 6,1 G | 6,2 G |

## 2. La cause

Le générateur représente toute valeur, même un enregistrement de 33 mots, par une valeur SSA de LLVM (un agrégat). À chaque appel d'une fonction récursive de `walk` :

- l'état de 13 mots part par valeur : 4 mots en registres, 9 empilés (`pushq`), relus par l'appelé à son entrée ;
- le résultat (33 mots) revient dans un tampon, que l'appelant relit champ par champ puis recopie dans son propre cadre, parce que ces valeurs vivent au-delà de l'appel suivant et que les registres manquent ; il le recopie encore dans son propre résultat.

rustc garde les grosses valeurs en mémoire : il passe leur adresse, l'appelé écrit son résultat là où l'appelant le gardera, et les copies restantes sont des `memcpy` vectoriels (la ligne « style d'Isocron »). Avec `&mut`, il n'y a même plus de copie de l'état : un seul état, modifié sur place.

## 3. Le changement

### Phase 1 : les grosses valeurs en mémoire aux frontières des fonctions

Une valeur d'un type agrégé « gros » (plus de 64 octets) passée à une fonction récursive ou rendue par elle :

- **le résultat** est écrit par l'appelé dans une place que lui donne l'appelant (`ptr %out`, son premier paramètre) ; l'appelant lie le résultat à cette place, sans le charger ;
- **un paramètre emprunté** (D102) est passé par son adresse (`ptr noalias readonly`) ; l'appelé le lit là où il s'en sert ;
- **une valeur en mémoire** (le résultat d'un appel, un paramètre, une de leurs parties) se lit par champ (`getelementptr` puis `load`) là où elle est lue, se passe par adresse, et se copie par `memcpy` quand il faut une copie ; partout ailleurs elle est chargée, comme aujourd'hui (le repli garde le programme juste) ;
- **un n-uplet rendu** (`[h2, s2]`) s'écrit directement dans la place du résultat.

Exclus de la phase 1 : les paramètres des fonctions qui se rappellent en queue (des boucles : leur état est dans les registres) et des appels en queue vers d'autres fonctions (D113 : l'appelé ne doit pas lire le cadre de l'appelant).

But mesuré : `walk` au niveau de Rust dans le style d'Isocron (1,2 s), sans plus d'instructions ailleurs. **Fait (D679)** : 2,02 s → 1,21 s.

### Phase 2 : la mise à jour sur place

Quand l'appelant ne relit plus une valeur après l'avoir donnée (c'est sa dernière lecture) et que l'appelé la rend mise à jour, les deux places peuvent n'en faire qu'une : l'appelé écrit l'état nouveau là où était l'ancien. C'est le `&mut` de Rust, déduit au lieu d'être écrit, et un seul état traverse toute la récursion. L'appelé doit lire tout ce qu'il lit de l'ancien état avant d'écrire le nouveau à la même place ; l'analyse de propriété (D96, D206, D637) dit déjà quelle lecture est la dernière.

But mesuré : `walk` au niveau de Rust avec `&mut` (0,88 s), puis en dessous. **Fait en partie (D680)**, avec l'écriture de chaque résultat à sa place : 1,21 s → 1,02 s (Rust par valeur 1,18 s, avec `&mut` 0,88 s) ; rtiow sur un fil 875 ms → 444 ms (Rust 356 ms).

### Suite : les boucles de rappels en queue (D681)

Une fonction dont les rappels en queue font une boucle (D102) copiait chaque paramètre passé par adresse dans une case de la boucle à son entrée, puis le résultat en retour : `hit` copiait deux fois les 112 octets de son générateur à chaque appel. La boucle garde désormais l'adresse donnée par l'appelant quand chaque rappel passe le paramètre tel quel ou que la fonction écrit son nouvel état par-dessus. Et LLVM ne sort plus de ces boucles ce qu'une branche calcule des paramètres inchangés (`llvm.licm.disable`) : il calculait les trois sinus d'une texture en damier à chaque appel de `textureValue`, quelle que soit la texture. rtiow (192 × 108) devant Rust dans les quatre cas : `sphere` sur un fil 1718 ms (Rust 1752), sur 8 fils 523 ms (689) ; `final` sur un fil 2667 ms (2695), sur 8 fils 646 ms (708). Le reste du corpus à ±0,4 %.

### Suite : les nœuds à la taille de leur variante, et les comptes des tâches (D682, D683)

Un nœud d'un type somme récursif réservait la place de toutes ses variantes (728 octets pour chaque objet de rtiow) ; il ne tient plus que celles de sa variante (D682 : le monde de la scène finale de 11,1 Mio à 4,6 Mio, Rust 3,6 Mio). Et une tâche dont les résultats ne tiennent aucun nœud ne compte plus que les nœuds que son itération a faits, marqués par elle : le monde partagé n'est plus écrit par les fils (D683). rtiow : `sphere` sur 8 fils 428 ms (Rust 676), sur un fil 1653 ms (1753) ; `final` sur 8 fils 622 ms (704), sur un fil 2453 ms (2706).

### Suite : la sortie et la mémoire (D685 à D688)

Les textes d'un `map` léger joints sont ajoutés un à un (D685), écrits dans leur fichier par parties au fur et à mesure (D686) ; le compilateur suit les valeurs possibles d'une expression et range un tableau de n-uplets d'entiers dans des types plus étroits (D687 : les couleurs de rtiow en octets) ; le premier morceau de chaque groupe de l'allocateur est en pages de 4 Kio (D688 : environ 2 Mio de moins pour tout programme). rtiow (384 × 216, protocole, Isocron 3e66f95) : `sphere` sur 8 fils 0,867 s (Rust 2,798), sur un fil 6,65 s (7,04) ; `final` 1,395 s (2,411) et 9,85 s (10,72) ; mémoire 1,3 Mio sur un fil (Rust 3,1). D689 a corrigé un défaut de D680 trouvé par l'éditeur d'isocron-ui (les branches d'un `if` écrit à sa place lisaient une variable reliée dans l'autre branche).

## 4. Ce qui reste sur les valeurs (dans cet ordre)

1. **Finir la phase 2** : la mise à jour sur place ne vaut aujourd'hui que pour les parties d'un état qui ne possèdent pas de mémoire (D680). Un état qui tient des textes, des tableaux ou des nœuds (l'état de l'éditeur d'isocron-ui, 37 champs ; la machine d'un interprète ; un analyseur et son texte) est encore recopié en partie. Mesures : l'éditeur, `walk`, un interprète.
2. **`walk` au niveau de Rust avec `&mut`** : 1,02 s contre 0,88 s ; mesurer d'abord les copies qui restent (instructions, lectures, écritures).
3. **Les cadres de pile** : le code généré ne marque la durée de vie d'aucune case (`llvm.lifetime`), les branches d'un grand `match` n'en partagent aucune (2,5 à 2,8 Ko pour `hit`, 180 Kio de pile touchés par fil de rtiow contre moins de 56 en Rust).
4. **Les calculs SPMD sur des enregistrements** (D140 ne prend que des scalaires) : plusieurs rayons, ou plusieurs requêtes, par instruction vectorielle.

Écarté à l'essai : découper à la main `hit` en une fonction par variante a été plus lent (2114 ms contre 1897), les très grandes fonctions ne sont pas la cause qu'elles semblaient.

## 5. La mesure

Chaque phase se mesure sur `bench/records/walk`, sur `bench/rtiow`, sur les programmes du Benchmarks Game et les noyaux de la preuve de concept, contre Rust et contre le plancher de la machine (`bench/machine.sh`). Une phase n'est gardée que si rien ne recule ailleurs.
