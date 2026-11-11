# Proposition : une bibliothèque d'interfaces graphiques en Isocron

Statut : **demandée par l'auteur le 2026-11-11** (« j'aimerais la reproduire mais avec isocron », puis « s'il manque des parties à isocron, développe les, oui », D631). Le modèle est GPUI, la bibliothèque graphique de l'éditeur Zed, sur laquelle Discord reconstruit son application de bureau. Ce document résume la lecture de GPUI, dit ce qui se transpose à Isocron et ce qui ne se transpose pas, puis liste les parties qui manquent à Isocron, dans l'ordre où elles seront faites.

## 1. GPUI, lu le 2026-11-11

Source lue : dépôt `zed-industries/zed`, commit `cd4902bf` du 2026-11-11, crates `gpui*`, `refineable`, `scheduler`, `sum_tree`. Rien n'a été compilé ni exécuté.

### 1.1 Les tailles

| Partie | Lignes |
| --- | --- |
| `gpui` (le cœur) | 103 890 |
| Linux (`gpui_linux`, X11 et Wayland) | 17 475 |
| Windows (`gpui_windows`, Win32, Direct3D 11, DirectWrite) | 14 700 |
| macOS et iOS (`gpui_macos`, `gpui_apple`, `gpui_ios`) | 19 992 |
| Rendu par wgpu et texte de Linux et du web (`gpui_wgpu`) | 8 101 |
| Web (`gpui_web`) | 7 887 |
| Le reste (macros, ordonnanceur, arbres, utilitaires) | 14 801 |
| **Total** | **186 846** |

Les dépendances externes, comptées dans leurs sources (`src/*.rs`) aux versions de `Cargo.lock` : la mise en page `taffy` 29 653 lignes ; l'accès au GPU `wgpu` (quatre crates) 157 572 et son compilateur de shaders `naga` 119 023 ; le texte `cosmic-text` 11 805, `harfrust` (mise en forme OpenType) 32 876, `swash` 26 513, `skrifa` et `read-fonts` 69 965 ; les chemins `lyon` 40 774 ; le SVG `resvg`, `usvg` et `tiny-skia` 37 767 ; la coupure des lignes `icu_segmenter` 10 399 ; le protocole X11 `x11rb` 170 688 (dont `x11rb-protocol`, 142 364, en grande partie engendré) ; Wayland 12 970 ; la boucle d'événements `calloop` 8 888.

### 1.2 Le modèle de l'application

- **Un seul propriétaire.** `App` possède tout : les entités (l'état de l'application, rangé par identifiant), les fenêtres, les valeurs globales, les abonnés. Une entité se modifie en la « prêtant » : elle sort de la table le temps de la mise à jour, et une mise à jour imbriquée de la même entité est une panne. Les rappels reçoivent le contexte en paramètre au lieu de le capturer.
- **Les effets en file.** `notify`, `emit` et les observateurs passent par une file vidée à la fin de la mise à jour la plus extérieure ; une fenêtre dont une vue a été notifiée est marquée à redessiner.
- **Un arbre d'éléments refait à chaque image**, comme un mode immédiat : chaque vue rend un arbre, en trois phases (mise en page, préparation, peinture). L'état qui doit survivre d'une image à l'autre est rangé sous un identifiant global de l'élément, et oublié s'il n'a pas servi à la dernière image. Une vue mise en cache rejoue les primitives de l'image précédente.
- **Les dépendances suivies automatiquement** : chaque entité lue pendant le dessin abonne la fenêtre, et une notification ne redessine que les fenêtres qui l'ont lue.
- **La souris** par une liste plate de zones de contact dans l'ordre de peinture ; **le clavier** par un arbre de répartition, des contextes de touches, des actions nommées et une table de raccourcis à expressions (`Editor && mode == x`).

### 1.3 Du style aux pixels

- **Le style** est une structure plate proche de CSS (`Style`), et ses « raffinements » ont des champs optionnels superposés pour les états (survol, actif, focus). Environ 3 100 méthodes du style sont engendrées par des macros à la Tailwind (`p_4`, `rounded_md`), 116 écrites à la main.
- **La mise en page** est confiée à `taffy` (flexbox, grille, blocs), mesurée en pixels de l'écran, arrondie par GPUI.
- **La scène** : une liste par sorte de primitive (quadrilatères, ombres, chemins, soulignements, glyphes, images), l'ordre de dessin donné par un R-tree (deux primitives qui ne se recouvrent pas partagent un rang et sont dessinées ensemble). Le découpage n'est fait que de rectangles ; l'opacité est multipliée dans les couleurs.
- **Le rendu** est sur le GPU : une instance par primitive, des shaders à champs de distance (rectangle arrondi à bords par côté, ombre par la fonction d'erreur, soulignement ondulé), les chemins tramés dans une texture intermédiaire avec anticrénelage, les glyphes dans un atlas (`etagere`) avec correction gamma.
- **Le texte** : chaque plateforme met en forme une ligne (cosmic-text et swash sous Linux, DirectWrite sous Windows, CoreText sous macOS) ; GPUI fait lui-même la coupure des lignes (`icu_segmenter`), le cache des lignes, les glyphes en quatre positions sous-pixel.

### 1.4 La couche système

- **Linux** : Wayland ou X11 selon les variables d'environnement. Les deux sont des protocoles parlés sur une socket Unix ; GPUI charge pourtant `libwayland-client`, `libxcb` et `libxkbcommon` (C), parce que Vulkan exige un pointeur C vers la connexion à l'affichage. Le presse-papiers X11 de GPUI parle déjà le protocole en Rust pur.
- **Windows** : tout passe par les DLL du système (`user32`, `imm32`, `dwmapi`, `d3d11`, `dxgi`, `dcomp`, `dwrite`), avec des objets COM que Windows rappelle. Windows n'a pas d'interface stable d'appels du système : ces DLL sont le seul chemin.
- **Web** : WebGPU ou WebGL2, des événements du DOM, aucun C.
- **Le GPU** : sous Linux, `libvulkan` et le pilote (ou EGL) sont du C chargé dans le processus ; aucun autre chemin vers le matériel n'existe.

## 2. Ce qui se transpose et ce qui ne se transpose pas

GPUI repose sur ce qu'Isocron n'a pas : un état modifiable partagé (`Rc<RefCell>`), des rappels rangés dans des structures (`Box<dyn FnMut>`), des destructeurs qui portent la sémantique (désabonnement, annulation), de l'asynchrone, des macros procédurales, et le GPU par du C. Isocron est pur, sans fermeture à l'exécution, ses effets dans des actions.

| GPUI | Isocron |
| --- | --- |
| Entités modifiables, contexte, file d'effets | **Un modèle à la Elm** : l'état de l'application est une valeur ; un événement devient un message (une valeur d'un type somme) ; `update(modèle, message)` rend le nouveau modèle ; `view(modèle)` rend l'arbre des éléments. La boucle d'événements est une action récursive terminale qui passe le modèle d'un tour au suivant, mis à jour sur place par l'unicité. |
| Rappels `cx.listener(...)` | Des messages rangés dans l'arbre (`onClick(Increment)`), ou des fonctions sans contexte dans un type somme (D153) : la défonctionnalisation qu'Isocron fait déjà. |
| Arbre refait à chaque image | Identique : `view` est une fonction pure. L'état propre aux éléments (défilement, champ en cours de saisie) vit dans le modèle de la bibliothèque, indexé par identifiant, comme chez GPUI. |
| `Style` et raffinements | Des records, et les méthodes de style sont des fonctions : `div().p(4).bg(red)` est déjà la syntaxe d'Isocron (`x.m(a)` est `m(x, a)`). Les états sont des `Option` de raffinements. |
| `taffy` | Une mise en page flexbox écrite en Isocron (fonction pure), la grille ensuite. |
| Scène par sortes de primitives | Des tableaux de records : rangés en colonnes par Isocron, ils sont déjà la disposition d'un moteur de rendu. |
| Rendu sur le GPU | **Un rendu sur le processeur, en Isocron** : l'image coupée en tuiles, chaque tuile ne lit que les primitives qui la touchent, les tuiles calculées en parallèle par `map`. Les formules des shaders de GPUI (champs de distance, ombre par la fonction d'erreur) se transposent telles quelles. Le résultat est le même au bit près sur toutes les cibles, quel que soit le nombre de cœurs : les images de référence des tests sont les mêmes partout. Que ce soit assez rapide (une image de 1920 × 1080 en quelques millisecondes) est une hypothèse, à mesurer dès les premières primitives. |
| Texte par la plateforme | Un lecteur de polices TrueType et OpenType, un tramage des contours, une mise en forme simple (table des caractères, avances, crénage) puis OpenType (GSUB, GPOS), la coupure des lignes d'Unicode (UAX 14), les polices trouvées dans `/usr/share/fonts`. Tout en Isocron. |
| `libwayland`, `libxcb`, `libxkbcommon` | Les protocoles Wayland et X11 parlés en Isocron sur une socket Unix ; l'image de la fenêtre dans une mémoire partagée (`wl_shm`) ou envoyée (`PutImage` de X11) ; la table du clavier de Wayland (format XKB en texte) lue par un analyseur écrit en Isocron, celle de X11 par `GetKeyboardMapping`. **Aucun C sous Linux.** |
| Windows (`user32`, Direct3D, DirectWrite) | Une fenêtre, sa procédure, sa boucle de messages et l'affichage d'une image par `user32` et `gdi32` : des DLL du système, l'exception déjà admise pour `kernel32` (D599). Le rendu et le texte restent ceux d'Isocron. À confirmer par l'auteur avant d'y venir. |
| Web | Plus tard : Isocron n'a pas de cible navigateur (seulement WASI). |

## 3. Deux dépôts

- **Dans Isocron** : ce qui touche au système, parce que seules les bibliothèques standard livrées avec `iscc` peuvent appeler les primitives (section 18, règle 2) : les primitives nouvelles et le module standard qui les offre, avec les règles du dépôt (tests, exemple du guide, microbenchmark et plancher en C).
- **Dans un dépôt à part** (`isocron-ui`, d'abord local ; sa publication sur GitHub attend le mot de l'auteur) : la bibliothèque elle-même, en Isocron pur, importée par `$ISC_PATH`. Elle vise une version d'`iscc` ; les manques qu'elle découvre sont corrigés dans Isocron.
- Le travail sur la bibliothèque peut commencer sans fenêtre : elle dessine d'abord dans un fichier image, et ses tests comparent des images.

## 4. Ce qui manque à Isocron, dans l'ordre

1. **Le module `unix`** (Linux) :
   - les sockets Unix en flux : `connect` et `listen` sur un chemin ou dans l'espace abstrait de Linux (`@nom`, celui de X11), les connexions ensuite lues, écrites et fermées par les primitives de `net` ;
   - l'envoi et la réception de descripteurs avec les octets (`SCM_RIGHTS`) : Wayland y passe ses mémoires partagées et la table du clavier ;
   - la mémoire partagée : un fichier en mémoire (`memfd_create`) projeté (`mmap` partagé), écrit et lu par des primitives qui vérifient leurs bornes, son descripteur envoyé au serveur d'affichage ; une mémoire reçue (la table du clavier) projetée en lecture ;
   - l'attente sur plusieurs descripteurs avec un délai (`poll`).

   Ces primitives sont **réalisées par le compilateur** : leurs appels du système sont émis dans le code du programme (une instruction `syscall`), comme la section 3.7 de la proposition du noyau le prévoit pour toutes les primitives de la v2. Aucune fonction n'est ajoutée au runtime en Rust (D622). L'interpréteur les fait aussi, avec le même résultat. Elles n'existent ni sous Windows ni sous WASI : un programme qui en atteint une est refusé à la compilation pour ces cibles (D324).
2. **Le temps de compilation d'une grande bibliothèque.** Les programmes de `json` et `http` se compilent déjà en 9 s. Une bibliothèque d'interfaces sera plus grande : l'item « hors du plan : le compilateur » de TODO.md devient utile ici, mesuré sur ses programmes.
3. **Windows** : des primitives de fenêtre réalisées par le compilateur sur `user32` et `gdi32` (créer une fenêtre, recevoir ses messages par une procédure émise par le compilateur, afficher une image). Après le mot de l'auteur.
4. Ce que la bibliothèque découvrira : chaque manque du langage sera noté dans DECISIONS.md et corrigé dans Isocron, mesuré comme le reste.

## 5. Plan de travail

1. Le module `unix` dans Isocron, ses tests (une socket Unix ouverte et acceptée dans le même programme, des descripteurs passés, une mémoire partagée écrite par un côté et lue par l'autre), son exemple du guide, son microbenchmark et son plancher en C.
2. Le dépôt `isocron-ui`, local : les primitives de la scène et le rendu sur le processeur dans une image, comparé à des images de référence, et mesuré.
3. Une fenêtre sous Wayland, puis sous X11, testées dans un serveur d'affichage sans écran (Weston en mode `headless`, Xvfb) dans Docker : une image affichée, la souris et le clavier reçus.
4. Le texte : une police lue, ses glyphes tramés, une ligne mise en forme et coupée.
5. La mise en page flexbox, les éléments (`div`, texte, image, liste), le style, les événements, le focus, le modèle à la Elm.
6. Windows, après le mot de l'auteur.

## 6. Questions pour l'auteur

1. Le nom du dépôt de la bibliothèque (provisoire : `isocron-ui`), et sa publication sur GitHub.
2. Windows : les appels à `user32` et `gdi32` sont-ils admis comme ceux de `kernel32` ? Lu comme oui du message de l'auteur du 2026-11-11 (« il faut prendre aussi en compte l'affichage sous windows », D635).
3. Le GPU : le rendu sur le processeur est proposé parce qu'il n'exige aucun C. Un rendu sur le GPU plus tard voudrait dire `libvulkan` et le pilote (du C) sous Linux, Direct3D (COM) sous Windows.
