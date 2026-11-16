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

## 4. Les autres causes connues (chantiers suivants)

- **Les comptes de références des données partagées en lecture** pendant un `map` parallèle : le monde de rtiow vit plus longtemps que le `map` et n'y change pas ; ses nœuds n'ont pas à être comptés dans les tâches (sur 8 fils, Rust perd les deux tiers de son travail sur ses `Arc`, Isocron 20 % sur ses comptes atomiques, D678).
- **Les très grandes fonctions** : tous les cas d'un `match` récursif fusionnés (3852 instructions, 2,9 Ko de pile pour le `hit` de rtiow), qui manquent de registres.
- **La fusion des productions** : un `map` vers des textes, joints puis écrits, gardé en mémoire tout entier (200 octets par pixel dans rtiow) au lieu d'être écrit au fur et à mesure.
- **Les calculs SPMD sur des enregistrements** (D140 ne prend que des scalaires) : plusieurs rayons, ou plusieurs requêtes, par instruction vectorielle.

## 5. La mesure

Chaque phase se mesure sur `bench/records/walk`, sur `bench/rtiow`, sur les programmes du Benchmarks Game et les noyaux de la preuve de concept, contre Rust et contre le plancher de la machine (`bench/machine.sh`). Une phase n'est gardée que si rien ne recule ailleurs.
