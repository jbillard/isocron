# Proposition : le noyau du runtime écrit en Isocron

Statut : **proposition soumise à l'auteur**. Rien de ce document n'est décidé ni implémenté. Elle révise la section 17 de SPEC.md après la décision D589 (plus de Rust du tout, noyau compris) et le relevé du runtime du 2026-11-09.

## 1. Le constat

Le noyau du runtime (allocateur, pool de threads, démarrage, entrées et sorties) ne peut pas être écrit avec les primitives de la section 17 telles qu'elles sont. Le relevé (chaque fait avec son fichier et sa ligne) donne huit manques :

1. **Les adresses.** L'allocateur aligne ses zones sur 2 Mio, place ses blocs à des décalages précis dans la page, chaîne ses blocs libres par des liens rangés dans les blocs eux-mêmes, et retrouve l'en-tête d'un bloc 64 octets avant son adresse. Le principe 3 (une primitive ne reçoit jamais d'adresse) l'interdit.
2. **Les données partagées sous verrou.** Les listes globales du tas, le contexte d'une tâche du pool et le tampon de `print` sont des données ordinaires protégées par des verrous. Le principe 5 n'admet que des accès atomiques.
3. **Les atomiques.** Le code actuel utilise cinq ordres (`Relaxed`, `Acquire`, `Release`, `AcqRel`, `SeqCst`), l'échange, le maximum, la comparaison faible, des barrières et l'indication d'attente active. La section 17 n'en nomme aucun.
4. **Les opérations sur une partie de région** : conseils au système sur une plage, déplacement de pages sans copie (`mremap`), projection de fichiers.
5. **L'état par thread** : sa création, sa mise à zéro, sa place, et ce qui doit s'exécuter à la fin du thread.
6. **Le passage de mémoire entre threads** : un bloc alloué par un thread et libéré par un autre, un tableau partagé en parts disjointes entre les tâches.
7. **Les chemins rapides du code généré**, qui lit aujourd'hui directement des structures du runtime (la liste libre des nœuds du thread, l'en-tête d'un bloc, le mode du tas).
8. **Windows et WebAssembly**, dont la section 17 ne dit rien.

## 2. Ce que le noyau doit garantir

- **Aucun Rust** dans le programme final (D589).
- **Au moins aussi rapide** que le noyau Rust actuel, partie par partie, mesuré (principe 6, D570).
- **Une sûreté énoncée précisément**, sans dialecte « de confiance » où tout serait permis. La proposition distingue deux choses :
  - ce que le langage **garantit par construction** même si le noyau contient une erreur ;
  - ce qui reste un **invariant du noyau**, vérifié par des tests et des outils, parce qu'aucun langage ne peut le garantir pour un allocateur (voir la section 5).

## 3. Le modèle proposé : l'arène confinée

### 3.1 Une seule réservation d'adresses

Au démarrage, le noyau réserve une plage d'adresses unique pour tout le tas, **l'arène** : par exemple 1 Tio, alignée sur 2 Mio, dont rien n'est engagé avant usage. Tout ce que l'allocateur gère (blocs, en-têtes, listes, caches des threads) vit dans l'arène.

### 3.2 Des poignées, jamais d'adresses

Dans les modules du noyau, la mémoire brute se désigne par une **poignée** : un entier qui situe un octet de l'arène. Les primitives de lecture et d'écriture (`load64(h)`, `store64(h, v)` et leurs variantes de 1, 2 et 4 octets) forcent la poignée dans l'arène avant d'y accéder.

- **Ce qui est garanti par construction** : un module du noyau ne peut ni lire ni écrire hors de l'arène. Une erreur de l'allocateur peut corrompre le tas, jamais la pile, le code ni une autre projection.
- **Le coût** : un masque par accès au pire. Deux variantes sont à mesurer avant de choisir (hypothèses tant que non mesurées) :
  - poignée de 64 bits masquée : une instruction `and` par accès ;
  - poignée de 32 bits comptée en blocs de 64 octets, avec une zone de garde après l'arène : aucun contrôle par accès, des liens deux fois plus petits, mais un tas limité à 256 Gio.
- Les liens des listes de blocs libres sont des poignées rangées dans les blocs : les listes chaînées restent possibles sans adresse.

### 3.3 Des accès définis même en cas de course

Une lecture et une écriture simultanées d'un même mot de l'arène ne sont jamais un comportement indéfini : les accès ordinaires à l'arène sont émis comme des accès atomiques sans ordre (`unordered`). D587 a mesuré qu'un accès atomique d'ordre `Relaxed` se compile en un simple `mov` sur x86-64 ; pour `unordered`, c'est une hypothèse à vérifier à l'étape 1. Une course ne peut donc donner qu'une valeur périmée, jamais une indéfinition. La course trouvée dans le tas actuel (D587) n'aurait pas été une faute de sûreté dans ce modèle.

À mesurer : D587 a montré que LLVM fusionne moins bien les accès atomiques (une comparaison en mémoire devient deux instructions). Le coût sur les chemins rapides de l'allocateur doit être chiffré.

### 3.4 Des atomiques complets et des verrous

- `atomicLoad`, `atomicStore`, `atomicAdd`, `atomicMax`, `atomicSwap`, `atomicCas` (forte et faible), sur 8, 32 et 64 bits, chacun avec un ordre constant parmi les cinq.
- `fence(order)`, `spinHint()`, `wait(h, expected)`, `wake(h, count)`.
- Les verrous sont écrits en Isocron au-dessus de ces primitives, comme aujourd'hui en Rust.

Le principe 5 devient : *aucune course ne peut produire un comportement indéfini*, au lieu de *tout accès partagé est atomique*.

### 3.5 L'état par thread

Chaque thread reçoit une zone de l'arène, mise à zéro, dont la poignée se lit en une instruction (registre de thread). Le pool, qui crée les threads, appelle la fonction de fin de thread du tas : plus besoin de clé pthread ni de rappel FLS.

### 3.6 Les chemins rapides

L'allocateur étant écrit en Isocron, ses fonctions courtes sont intégrées au code du programme comme n'importe quelle fonction du prélude (c'est ce qui a rendu `parseI64` plus rapide que sa version Rust). Le générateur de code ne connaît plus la disposition des structures du tas : il appelle `alloc`, `free`, `nodeAlloc`, `nodeFree`, et l'intégration fait le reste.

### 3.7 Les appels système et les threads

- Fichiers et horloge : les `sys*` de la section 17, complétés (`sysSleep`, nombre de processeurs, variables d'environnement).
- Mémoire : `arenaCommit`, `arenaRelease`, `arenaAdvise(h, bytes, advice)`, `arenaMove(from, to, bytes)` (déplacement de pages sans copie), tous bornés à l'arène.
- Threads : `spawn(stackBytes, entry)` et `join`. Le changement de pile et le démarrage (`isc_start`) restent quelques lignes d'assembleur émises par le compilateur.
- Windows et WebAssembly : chaque primitive a sa réalisation par cible dans le compilateur. Sous Windows, cela veut dire des appels aux fonctions de `kernel32` : il n'existe pas d'autre moyen d'obtenir de la mémoire ou un thread sur ce système.

## 4. Ce qui change dans SPEC section 17

- **Principe 3**, reformulé : une primitive ne reçoit jamais d'adresse de la machine ; la mémoire brute se désigne par des poignées confinées à l'arène.
- **Principe 5**, reformulé : aucune course ne produit de comportement indéfini ; les verrous et les protocoles sans verrou s'écrivent avec des atomiques ordonnés.
- **Table des primitives**, révisée selon la section 3 de ce document.
- **Ordre de travail**, point 6 : les expressions régulières et les grands entiers ne restent plus en Rust (D589) ; ils viennent après le noyau.

## 5. Ce qui reste un invariant du noyau

Deux choses ne peuvent être garanties par aucun système de types simple, et resteraient des invariants vérifiés par des tests :

1. **L'allocateur ne donne jamais deux fois la même mémoire.** `arrayOf(h, count)` transforme un bloc de l'arène en tableau du programme. Si l'allocateur se trompe, deux tableaux se recouvrent. C'est vrai de tout allocateur, en Rust comme ailleurs.
2. **Une tâche publiée aux threads du pool vit jusqu'à la fin de la boucle.** Le contexte d'une tâche est dans la pile de l'appelant ; le protocole du pool garantit que l'appelant attend.

Moyens de vérification proposés : l'allocateur vérifié (`ISC_ALLOC=check`, D546) réécrit sur les mêmes primitives, les tests sous Valgrind avec le tas du système, et les tests de charge à plusieurs threads déjà dans la suite.

## 6. Plan de travail, chaque étape mesurée contre Rust

1. **Un prototype de mesure** : les petites classes de l'allocateur (les plus chaudes) sur l'arène, avec les deux variantes de poignée. Si aucune n'égale le chemin rapide actuel (27 instructions pour `isc_alloc`, 36 pour `isc_free`), le modèle est à revoir avant d'aller plus loin.
2. L'allocateur complet : petits blocs et caches des threads, blocs moyens, grands blocs.
3. Le pool de threads et la création de threads.
4. Les entrées et sorties : `print` et son tampon, fichiers, entrée standard.
5. Le démarrage et les pannes.
6. Les copies et comparaisons de blocs (`memcpy`, `memset`, `memcmp`), aujourd'hui fournies par Rust.
7. Windows, puis WebAssembly.
8. Les expressions régulières et les grands entiers.

Les fonctions pures de texte et de nombres continuent en parallèle : elles ne dépendent pas de ce modèle.

## 7. Questions pour l'auteur

1. **Le modèle de l'arène confinée convient-il**, avec ses deux invariants de la section 5 ?
2. **La limite de taille du tas** : une arène de 1 Tio d'adresses réservées (poignées de 64 bits) ou de 256 Gio (poignées de 32 bits, sans contrôle par accès) ? La mesure de l'étape 1 éclairera ce choix.
3. **Windows** : les appels à `kernel32` sont-ils acceptables comme seule exception à « pas d'appel de C » ?
