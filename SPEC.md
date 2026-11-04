# Isocron — spécification du noyau

Oct 31, 2026 · @Joël Billard

## 1. Objectifs et principes

Le noyau vise un seul résultat : battre C et Rust en vitesse brute sur CPU, avec une syntaxe simple et des performances déterministes au sens défini ci-dessous.

Nom : **Isocron** (« de durée égale »). C'est un nom d'intention : le langage supprime les sources de variation qui dépendent de lui, pas celles du matériel. Extension de fichier : `.isc`.

### Ce que « performances déterministes » signifie

| Garanti par le langage | Non garanti |
| --- | --- |
| aucun ramasse-miettes, aucun JIT | le temps d'exécution réel, qui dépend des caches, de la fréquence du CPU, de l'OS et de la mémoire |
| aucune allocation, copie ou vérification de bornes cachée : toutes figurent dans le memory report | le temps d'une `action` d'entrée/sortie |
| découpage du travail fixé par la spécification (blocs de B éléments), indépendant du nombre de cœurs |  |
| résultats identiques au bit près, quels que soient la cible et le nombre de cœurs (section 10) |  |

### Langue

- Le langage est entièrement en anglais : mots-clés, primitives, bibliothèque standard, messages du compilateur, noms de crates.
- Ce document est rédigé en français. Les termes techniques gardent leur nom anglais usuel quand il existe (inlining, fold, scan, destination-passing).

### Périmètre de la version 0

- Paradigme : fonctionnel pur, évaluation stricte.
- Données : scalaires, tableaux contigus à taille typée, tuples, records, types somme.
- Cible : CPU x86-64, puis ARM64.
- Hors périmètre : GPU, `@requireVectorized`, concurrence écrite par le programmeur, fonctions stockées dans des structures de données.

### Principes directeurs

1. Le programmeur décrit **quoi** calculer ; le langage interdit ce qui empêche le compilateur de décider **comment**.
2. Aucun coût caché : toute copie, allocation ou comptage de références non garanti apparaît dans le memory report.
3. Les garanties de performance sont optionnelles, écrites par le programmeur, et vérifiées à la compilation.
4. Par défaut, un programme donne le même résultat au bit près, quel que soit le nombre de cœurs.
5. Toute garantie repose sur une règle écrite dans cette spécification, jamais sur une heuristique du compilateur.

### Convention de lecture

« Doit » signifie obligatoire pour toute implémentation. « Peut » désigne une liberté laissée au compilateur. Une question non tranchée figure en section 16 ; elle n'est jamais décidée en silence dans le code. Les choix marqués « décision » dans ce document sont des choix de conception, modifiables.

## 2. Décisions de conception

Les choix ci-dessous sont pris ; ceux marqués « recommandation » restent à confirmer.

| Sujet | Décision | Section |
| --- | --- | --- |
| Objectif | Battre C/Rust en vitesse brute ; syntaxe simple ; performances déterministes | 1 |
| Nom | Isocron, fichiers `.isc` | 1 |
| Langue du langage | Anglais (mots-clés, primitives, messages) | 1 |
| Démarche | Un POC d'abord, puis des programmes comparés à C, Rust, Go, Zig | 14, 15 |
| Paradigme | Fonctionnel pur, seul | 5 |
| Évaluation | Stricte (appel par valeur) | 5 |
| Données | Tableaux contigus par défaut, valeurs à plat, monomorphisation | 4 |
| Mémoire | Unicité inférée, `requireUnique`, comptage de références en secours, aucun ramasse-miettes | 8 |
| Exigences v0 | `requireUnique`, `requireNoAlloc`, `requireFused`, `requireTail` | 9 |
| Exigence reportée | `@requireVectorized` | 16 |
| Flottants | IEEE 754, reproductibles par défaut ; mode `allowReassoc` optionnel | 10 |
| Fonctions d'ordre supérieur | Résolues à la compilation, aucune fermeture allouée | 11 |
| Effets | Isolés dans les `action`, aux bords du programme | 12 |
| Cible | CPU x86-64, puis ARM64 ; GPU non exclu par la conception | 1 |
| Implémentation | Compilateur en Rust, back-end LLVM (recommandation) | 13 |

## 3. Syntaxe

La syntaxe reprend celle de TypeScript (D148) : accolades, `;` obligatoires, `const`, `return`, lambdas `=>`, types après `:`. Un lecteur qui connaît TypeScript ou JavaScript lit un programme Isocron sans apprentissage. Les écarts viennent de la sémantique : les valeurs ne changent jamais (pas de `let` ni d'affectation), les tableaux portent leur taille dans leur type, et un `function` est pur.

### Lexique

- Commentaires : `//` jusqu'à la fin de la ligne, `/* ... */` sur plusieurs lignes.
- Identifiants : lettres ASCII, chiffres et `_`, sans chiffre en tête.
- Littéraux : entiers `42`, `0xFF`, `1_000_000` ; flottants `3.14`, `1e-9` ; booléens `true`, `false` ; chaînes `"..."` ou `'...'` ; gabarits `` `n = ${n}` `` (D150), sur plusieurs lignes si besoin ; entiers de taille quelconque `123n`, `0xffn` (`bigint`, D156) ; expressions régulières `/source/drapeaux` (D157), là où une expression commence (au début, après `return` ou après tout signe sauf `)`, `]`, `}` et la fin d'un gabarit ; ailleurs `/` est la division), drapeaux parmi `g`, `i`, `m`, `s`, `u`. Le type des littéraux numériques est fixé par le contexte (section 4).
- Suffixes de type facultatifs : `42u8`, `1.0f32`.
- Un opérateur entre parenthèses est une fonction : `(+)`, `(*)`, `(&&)`.
- Mots réservés : `function`, `action`, `declare`, `type`, `const`, `return`, `if`, `else`, `match`, `import`, `export`, `true`, `false`, `size`, `with`. `let`, `var` et `fn` sont réservés pour donner un message clair.
- Décorateurs : `@requireNoAlloc`, `@requireFused`, `@requireTail`, `@allowReassoc`, placés avant `function`.

### Grammaire (EBNF)

```ebnf
program      = { import } { declaration } ;
import       = "import" ( "*" "as" IDENT
                        | "{" IDENT [ "as" IDENT ] { "," IDENT [ "as" IDENT ] } "}" )
               "from" STRING ";" ;
declaration  = { decorator } [ "export" ]
               ( function | action | type_decl | const_decl | declare ) ;
decorator    = "@" ( "requireNoAlloc" | "requireFused"
                   | "requireTail" | "allowReassoc" ) ;
function     = "function" IDENT [ generics ] "(" [ params ] ")" ":" type block ;
action       = "action" IDENT "(" [ params ] ")" ":" type block ;
declare      = "declare" "function" IDENT "(" [ params ] ")" ":" type ";" ;
type_decl    = "type" IDENT [ generics ] "="
               ( type | [ "|" ] variant { "|" variant } ) ";" ;
variant      = IDENT [ "(" type { "," type } ")" ] ;
const_decl   = "const" IDENT [ ":" type ] "=" expr ";" ;
generics     = "<" IDENT [ "extends" CLASS ] { "," IDENT [ "extends" CLASS ] } ">" ;
CLASS        = "Num" | "Ord" | "Eq" | "Key" ;
params       = param { "," param } ;
param        = IDENT ":" ( type | "size" ) ;

type         = base_type { "[" dim "]" } ;
dim          = size_expr | IDENT "<=" size_expr ;
base_type    = SCALAR | "string" | "bigint" | "void"
             | IDENT [ "." IDENT ] [ "<" type { "," type } ">" ]
             | "[" [ type { "," type } ] "]"
             | "{" IDENT ":" type { "," IDENT ":" type } "}"
             | "(" [ fparam { "," fparam } ] ")" "=>" type
             | "(" type ")" ;
fparam       = [ IDENT ":" ] type ;
SCALAR       = "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
             | "f32" | "f64" | "boolean" ;
size_expr    = size_term { ( "+" | "-" ) size_term } ;
size_term    = size_atom { "*" size_atom } ;
size_atom    = INT | IDENT | "(" size_expr ")" ;

block        = "{" { statement } "}" ;
statement    = "const" pattern [ ":" type ] "=" expr ";"
             | "return" [ expr ] ";"
             | "if" "(" expr ")" branch [ "else" branch ]
             | expr ";" ;
branch       = block | statement ;

expr         = lambda | cond ;
lambda       = ( IDENT | "(" [ lparam { "," lparam } ] ")" ) "=>" ( expr | block ) ;
lparam       = IDENT [ ":" type ] ;
cond         = coalesce [ "?" expr ":" expr ] ;
coalesce     = binary [ "??" coalesce ] ;
binary       = power { BINOP power } ;
BINOP        = "||" | "&&" | "|" | "^" | "&" | "==" | "===" | "!=" | "!=="
             | "<" | "<=" | ">" | ">=" | "<<" | ">>"
             | "+" | "-" | "*" | "/" | "%" ;
power        = unary [ "**" power_right ] ;
power_right  = unary [ "**" power_right ] ;
unary        = ( "-" | "!" ) unary | postfix ;
postfix      = primary { "(" [ args ] ")" | "[" expr "]" | "." IDENT
                       | "as" "const" } ;
primary      = LITERAL | STRING | BIGINT | REGEX | template | IDENT
             | "(" ")" | "(" BINOP ")" | "(" expr ")" | "[" [ args ] "]"
             | record | match | "requireUnique" "(" expr ")"
             | "new" ( "Map" | "RegExp" ) ;
template     = "`" TEXT { "${" expr "}" TEXT } "`" ;
record       = "{" field { "," field } [ "," ] "}"
             | "{" "..." expr "," field { "," field } [ "," ] "}" ;
field        = IDENT [ ":" expr ] ;
match        = "match" "(" expr ")" "{" arm { "," arm } [ "," ] "}" ;
arm          = pattern "=>" ( expr | block ) ;
args         = expr { "," expr } [ "," ] ;
pattern      = "_" | IDENT [ "." IDENT ] | LITERAL | "-" NUMBER | "(" ")"
             | "[" [ pattern { "," pattern } ] "]"
             | IDENT [ "." IDENT ] "(" [ pattern { "," pattern } ] ")"
             | "{" IDENT [ ":" pattern ] { "," IDENT [ ":" pattern ] } "}" ;
```

- **Déclarations de type** (D159) : après `=`, un type qui commence par `{`, `[`, `(`, un type prédéfini (`f64`, `string`, `boolean`, `bigint`, `void`...) ou un `IDENT` suivi de `<` ou `[` fait de la déclaration un alias, comme en TypeScript : l'alias est ce type, remplacé partout avant la vérification, et une variable de taille qu'il contient devient une variable de la signature qui l'emploie ; sinon la déclaration est un type somme (`type A = B;` est un type somme à un constructeur). Un alias qui se contient lui-même est une erreur.
- **Constantes** (D159) : un `const` au niveau des déclarations est une valeur lisible de toute fonction. Sans type, sa valeur est un littéral ou un tableau de littéraux, du type du `number` de TypeScript pour les nombres (`i64` ou `f64`) ; sinon le type est écrit. Une variable locale du même nom la masque ; une constante qui dépend d'elle-même est une erreur.
- **Décorateurs** : seulement devant `function`.
- **Corps** : un `function` se termine par `return` sur chaque chemin. Un `if` dont une branche contient `return` choisit la valeur : la suite du corps continue la branche qui ne retourne pas (`if (n < 2) return n;` en tête de fonction est une garde). Dans une `action`, un `if` sans `return` est une instruction ; la dernière instruction d'une action sans `return` donne `void`.
- **Lambda ou parenthèse** : après `(`, le parser cherche la `)` correspondante ; si elle est suivie de `=>`, c'est une lambda.
- **Méthodes** : `x.m(a, b)` est `m(x, a, b)`, résolu par le vérificateur de types quand `x` n'est ni un record ni un module (`xs.map(f)`, `x.sum()`) ; `m` y désigne toujours une fonction, même si une variable locale du même nom existe (D158). `m.size` est le nombre de clés d'un `Map`. `x.length` est `len(x)`, `x.slice(a, b)` la tranche de `a` inclus à `b` exclu, `x.with(i, v)` la mise à jour (section 8), `{ ...r, f: v }` la mise à jour d'un champ.
- **Tuples** : `[a, b]` est un tableau ; `[a, b] as const` est un tuple, de type `[A, B]`. Là où un tuple est attendu (type annoncé, résultat déclaré), `[a, b]` suffit.
- **Tableaux à deux dimensions** : `T[r][c]` est un tableau de `r` lignes de `c` éléments ; `x[i]` est une ligne.
- **Motifs** : un `IDENT` suivi de `(` est un constructeur de type somme ; seul, c'est une variable, sauf s'il nomme un constructeur sans argument.
- **Modules** (D139, D150) : `import * as m from "./m";` donne accès à ce que le module exporte sous les noms `m.f`, `m.T`, `m.C` ; `import { f, g as h } from "./m";` rend ces noms utilisables sans préfixe, après les variables locales. Un chemin commençant par `./` ou `../` est relatif au fichier qui importe ; un nom nu est cherché dans `$ISC_PATH`. Seul ce qui est déclaré avec `export` est visible d'un autre fichier ; un module est un fichier, lu une seule fois.
- **Noms** (D151) : la bibliothèque suit les conventions de TypeScript, en camelCase (`readF64`, `parseCsvF64`, `printBytes`, `@requireFused`), et reprend ses noms là où le sens est le même (`String(x)`, `x.toString()`, `Math.sqrt`, `Math.min`, `Math.PI`) ; un nom de TypeScript n'est repris qu'avec son sens : `parseFloat("1,5")` vaut 1 comme en TypeScript, `parseF64`, strict, refuse ce texte (D156). `parseI64` et `parseF64` sont des fonctions pures, comme `Number`, `parseFloat` et `parseInt`.
- **Gabarits** : `` `a${e}b` `` est la concaténation des textes et des insertions, chacune écrite comme `print` l'écrit (scalaire ou chaîne).
- **`??` et `new`** (D158) : `a ?? b` est la valeur de `Some` dans l'`Option` `a`, sinon `b` (le `??` de TypeScript, `None` tenant lieu d'`undefined`) ; `new Map(e)` et `new RegExp(s, f)` sont les appels `Map(e)` et `RegExp(s, f)` ; `new` ne construit rien d'autre.

### Priorité des opérateurs

Les niveaux sont ceux de TypeScript (norme ECMAScript).

| Niveau | Opérateurs | Associativité |
| --- | --- | --- |
| 1 (le plus faible) | `? :` | droite |
| 2 | `??` | droite |
| 3 | `\|\|` | gauche |
| 4 | `&&` | gauche |
| 5 | `\|` | gauche |
| 6 | `^` (ou exclusif bit à bit) | gauche |
| 7 | `&` | gauche |
| 8 | `==` `===` `!=` `!==` | aucune |
| 9 | `<` `<=` `>` `>=` | aucune |
| 10 | `<<` `>>` | gauche |
| 11 | `+` `-` | gauche |
| 12 | `*` `/` `%` | gauche |
| 13 | `-` unaire, `!` | — |
| 14 | `**` (puissance) | droite |
| 15 (le plus fort) | appel, indice, `.`, `as const` | gauche |

Décisions :
- comme en TypeScript, `-x ** 2` est refusé : il faut écrire `-(x ** 2)` ou `(-x) ** 2` ; `2 ** -1` est accepté ;
- les comparaisons ne s'enchaînent pas : `a < b < c` est une erreur (en TypeScript, elle compare un booléen à `c`) ;
- `===` et `!==` sont `==` et `!=` : l'égalité porte toujours sur les valeurs, tableaux compris ;
- `&`, `|` et `^` sont sous les comparaisons, comme en TypeScript : `a & b == c` est `a & (b == c)`, une erreur de type pour des entiers.

### Exemple complet

```
// Simple statistics on a file of floats
@requireNoAlloc
function average(x: f64[n]): f64 {
    return sum(x) / f64(n);
}

function stdDev(x: f64[n]): f64 {
    const m = average(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

function histogram(values: u8[n]): u32[256] {
    return fold(values, zeros(), (c, v) => requireUnique(c.with(v, c[v] + 1)));
}

action main(args: string[k]): i32 {
    const data = readF64(args[0]);
    print(stdDev(data));
    return 0;
}
```

Dans `histogram`, le littéral `1` prend le type `u32` de `c[v]`, et `v` (un `u8`) est accepté comme indice (section 4).

## 4. Système de types

Chaque valeur a un type connu à la compilation, et aucune conversion entre valeurs n'est implicite.

### Scalaires

| Type | Taille | Remarque |
| --- | --- | --- |
| `i8` `i16` `i32` `i64` | 1 à 8 octets | entiers signés, complément à deux |
| `u8` `u16` `u32` `u64` | 1 à 8 octets | entiers non signés |
| `f32` `f64` | 4 et 8 octets | IEEE 754 binary32 et binary64 (section 10) |
| `boolean` | 1 octet | `true`, `false` |
| `void` | 0 octet | type unité, résultat d'une action sans valeur |
| `string` | opaque | texte UTF-8 immuable ; valeur comme les autres (D134) |
| `bigint` | opaque | entier de taille quelconque, littéraux `123n` (D156) ; ne se mélange pas aux autres nombres |

- **Littéraux** : un littéral sans suffixe prend le type exigé par son contexte (`c[v] + 1` avec `c[v]: u32` donne `1: u32`). Si le contexte ne fixe rien : `i64` pour un entier, `f64` pour un flottant. Un littéral hors de la plage du type est une erreur de compilation. Ce n'est pas une conversion : le littéral est typé directement.
- **Débordement entier** : arithmétique modulaire, définie. C'est le comportement de Rust en mode release ; en C, le débordement signé est indéfini, ce qui autorise certaines optimisations (section 16).
- **Division entière** (décision) : quotient tronqué vers zéro ; `a % b` a le signe de `a`. Division ou reste par zéro : panne. `MIN / -1` donne `MIN` et `MIN % -1` donne `0` (arithmétique modulaire).
- **Puissance entière** : `a ** k` exige `k >= 0`, sinon panne ; calculée modulo 2^bits comme le reste de l'arithmétique entière.
- **Conversions explicites** : `f64(n)`, `i32(x)`. D'un flottant vers un entier : troncature vers zéro, saturation aux bornes, NaN donne 0.

### Tableaux

- `T[n]` : `n` éléments de type `T`, contigus en mémoire. `n` est une constante (`u32[256]`) ou une variable de taille (`f64[n]`).
- **Indices** : tout type entier est accepté comme indice. Un indice négatif ou trop grand est une panne.
- **Variables de taille** : une variable de taille apparaissant dans le type d'un paramètre est liée par l'argument reçu, et utilisable comme valeur `i64`.
- **Paramètres de taille** : un paramètre déclaré `k: size` est une valeur `i64` positive ou nulle utilisable dans les types : `function window<T>(x: T[n], k: size): T[n - k + 1][k]`.
- **Tailles existentielles** : une variable de taille qui n'apparaît que dans le type du résultat est fixée à l'exécution par la fonction : `readF64(path)` est de type `f64[k]`. La forme bornée `T[k <= n]` (résultat de `filter`) précise en plus que `k` ne dépasse pas `n`.
- **Expressions de taille** : sommes et produits de constantes et de variables de taille (`T[n - k + 1]`, `T[2 * n]`). Deux tailles sont égales à la compilation si leurs formes polynomiales normalisées sont identiques ; sinon, un test est inséré à l'entrée de la fonction. Un échec est une panne (section 5) ; chaque test restant figure dans le memory report.
- **Tableaux multidimensionnels** `T[n][m]` (`n` lignes de `m` éléments) : réguliers, stockés ligne par ligne dans un seul bloc.
- Un tableau de tuples ou de records doit être stocké en colonnes, une colonne par champ (structure of arrays). Le choix automatique de la disposition est reporté (section 16).

### Tuples et records

- Tuple de type `[f64, i32]`, écrit `[2.5, 3] as const` ; record `{ price: f64, volume: u32 }`, écrit `{ price: 2.5, volume: 3u32 }`.
- Représentés à plat : aucun pointeur, champs côte à côte.
- Mise à jour fonctionnelle : `{ ...r, price: 2.0 }`.

### Types somme

```
type Option<T> = None | Some(T);

type Result<T, E> = Ok(T) | Err(E);
```

- Représentation : étiquette plus charge utile, à plat, sans allocation.
- Types récursifs (listes, arbres) : autorisés, alloués sur le tas avec comptage de références. Chacun apparaît dans le memory report.
- `Option<T>` et `Result<T, E>` sont dans la bibliothèque standard ; un programme qui déclare l'un de ces noms ou de leurs constructeurs les remplace (D138, D158).

### Types de la bibliothèque

- `RegExp` (D157) : une expression régulière, compilée par un moteur en temps linéaire (pas de référence arrière ni d'assertion de contexte) ; une expression littérale invalide est une erreur de compilation. Ni affichée ni comparée.
- `Map<K, V>` (D158) : un dictionnaire immuable dont les clés gardent l'ordre de leur première insertion ; construit d'un bloc en O(n + m log m) pour n entrées et m clés distinctes (une table de hachage, puis le tri des m clés, D167), lu en O(log m). Ni affiché, ni comparé, ni rangé dans un tableau ou un constructeur.
- Un alias `type Nom = T;` n'est pas un nouveau type : c'est `T` (D159).

### Génériques et classes

- Paramètres de type : `function first<T>(x: T[n]): T`.
- Monomorphisation obligatoire : une version compilée par combinaison de types utilisée, aucune valeur emballée (boxing).
- Classes prédéfinies, non extensibles en v0 : `Num` (+ − × ÷, élément neutre `0` et `1`), `Ord` (comparaisons : entiers, flottants, booléens, chaînes, `bigint`), `Eq` (égalité), `Key` (clés d'un `Map` : entiers, booléens, chaînes, `bigint` ; ni les flottants, dont l'égalité n'en est pas une pour NaN, D158). Exemple : `function sum<T extends Num>(x: T[n]): T`.

### Inférence

- Signature obligatoire pour toute fonction de premier niveau : messages d'erreur clairs et vérification des tailles.
- Types des `const` et des lambdas inférés localement ; une annotation reste possible (`const a: u8 = 250;`, `(v: f64) => v * 2.0`).

## 5. Sémantique d'évaluation

Le langage est strict et pur : chaque argument est entièrement calculé avant l'appel, et une fonction ne fait rien d'autre que produire son résultat.

- **Strict** : appel par valeur. Aucune évaluation différée, aucun thunk.
- **Pur** : une `function` ne lit ni n'écrit aucun état extérieur. Mêmes entrées, même résultat au bit près (hors `allowReassoc`, section 10).
- **Ordre libre** : dans une `function`, l'ordre d'évaluation des sous-expressions indépendantes n'est pas spécifié. Il n'est pas observable, sauf pour le choix de la panne signalée. Dans une `action`, l'ordre est imposé (section 12).
- **Pannes (panic)** : accès hors bornes, division entière par zéro, exposant entier négatif, tailles incompatibles, dépassement de pile, erreur d'entrée/sortie. Une panne arrête le programme avec un message et la position dans le source. Si plusieurs pannes sont possibles, celle qui est signalée n'est pas spécifiée ; l'arrêt est garanti.
- **Bornes** : chaque indexation est vérifiée. Le compilateur doit supprimer les vérifications qu'il prouve inutiles ; celles qui restent figurent dans le memory report.
- **Égalité** : tableaux, tuples et records se comparent par valeur. Il n'existe pas de notion d'identité.

## 6. Primitives de tableaux

Toutes les boucles s'écrivent avec ces primitives ; le tableau est toujours le premier argument, pour permettre l'écriture en méthode `x.f(…)` (section 3).

| Primitive | Type | Sémantique | Parallèle |
| --- | --- | --- | --- |
| `map(x, f)` | `(T[n], (T) => U) => U[n]` | applique `f` à chaque élément | oui |
| `zip(a, b)` | `(A[n], B[n]) => [A, B][n]` | associe deux tableaux ; une vue, coût nul en colonnes | — |
| `reduce(x, op, e)` | `(T[n], (T, T) => T, T) => T` | combine selon la forme fixe de la section 10 | oui |
| `scan(x, op, e)` | `(T[n], (T, T) => T, T) => T[n]` | cumul inclusif, défini de gauche à droite | seulement si `op` est un opérateur prédéfini associatif sur entiers ou booléens ; sinon séquentiel |
| `filter(x, p)` | `(T[n], (T) => boolean) => T[k <= n]` | garde les éléments où `p` est vrai, dans l'ordre | oui |
| `fold(x, init, f)` | `(T[n], A, (A, T) => A) => A` | parcours de gauche à droite | non, par définition, sauf sous la règle P (ci-dessous) |
| `window(x, k)` | `(T[n], k: size) => T[n - k + 1][k]` | fenêtres glissantes ; une vue, jamais copiée ; panne si `k > n` | oui |
| `x.slice(a, b)` | `(T[n], i64, i64) => T[b - a]` | sous-tableau ; une vue ; panne si `a < 0`, `a > b` ou `b > n` (sans les conventions de TypeScript pour les bornes négatives ou trop grandes) | — |
| `iota(n)` | `(n: size) => i64[n]` | `[0, 1, …, n - 1]` | oui |
| `fill(n, v)` | `(n: size, T) => T[n]` | `n` copies de `v` | oui |
| `zeros()` | `() => T[n]` avec `T extends Num` | taille déduite du type attendu | oui |
| `x[i]` | `(T[n], entier) => T` | lecture, bornes vérifiées | — |
| `x.with(i, v)` | `(T[n], entier, T) => T[n]` | nouveau tableau où `i` vaut `v` (section 8) | — |

### Précisions

- **Opérateurs prédéfinis associatifs** sur entiers et booléens : `+`, `*` (arithmétique modulaire), `min`, `max`, `&&`, `||`. Ce sont les seuls pour lesquels `scan` est parallélisé, car un cumul parallèle n'est identique au cumul séquentiel que si `op` est associatif.
- **Élément neutre de `reduce`** : `e` devrait être neutre pour `op`, mais le compilateur ne peut pas le vérifier. S'il ne l'est pas, le résultat reste celui, déterministe, de la forme de la section 10 ; il diffère alors d'un parcours séquentiel.
- **Règle P, accumulation privatisée** (décision) : un `fold(x, init, f)` dont la fonction a la forme `(c, v) => c.with(g(v), c[g(v)] + h(v))` (ou `h(v) + c[g(v)]`), où les éléments de `c` sont des entiers et où ni `g` ni `h` n'utilisent `c`, peut être calculé en parallèle. `x` est découpé en morceaux consécutifs ; chaque morceau est accumulé dans une copie privée, remplie de zéros et de la taille de `init` ; les copies sont ensuite ajoutées à `init` élément par élément. L'addition entière modulo 2^bits est associative et commutative : le résultat est identique à celui du `fold` séquentiel pour tout découpage, donc pour tout nombre de cœurs. Les indices restent vérifiés : un indice hors bornes est une panne, à la position de la lecture `c[g(v)]`. La règle ne s'applique ni aux accumulateurs flottants (leur addition n'est pas associative) ni à une autre forme de mise à jour. C'est le cas de l'histogramme. La forme générale est `scatter(dest, idx, vals, op)` (D111) : les valeurs d'un même indice sont combinées dans l'ordre de k, quel que soit l'opérateur ; elle n'est parallèle que sous la forme de la règle P.

### Opérateurs élément par élément

- `a + b`, `a * b` sur deux tableaux de même taille ; `x - m` avec un scalaire `m` diffusé à chaque élément (broadcast).
- Équivalents à un `map` et soumis aux mêmes règles de fusion (section 7).

### Bibliothèque standard (écrite en Isocron)

- `sum(x) = reduce(x, (+), 0)`, `product`, `min`, `max` (sémantique flottante en section 10).
- `count(x, p)`, `mean(x)`, `dot(a, b) = sum(a * b)`.
- Fonctions mathématiques : `sqrt`, `fma`, `pow`, `exp`, `log`, `sin`, `cos` (section 10), et l'espace `Math` de TypeScript : `floor`, `ceil`, `trunc`, `round`, `abs`, `tan`, `asin`, `acos`, `atan`, `atan2`, `min`, `max`, `PI`, `E` (D151, D155).
- Tri stable `sort`, `sortBy`, `toReversed` ; nombres aléatoires à graine `random(seed, i)` et `randomInt`, les mêmes bits partout (D155).
- Chaînes et texte (D134, D135, D155) : `slice`, `indexOf`, `includes`, `startsWith`, `endsWith`, `replace`, `replaceAll`, `toUpperCase`, `toLowerCase`, `padStart`, `padEnd`, `repeat`, `trim`, `split`, `join`, `toFixed`, `String`, `Number`, `parseFloat`, `parseInt`, `parseI64`, `parseF64` ; entrée standard `readStdin`, `readStdinText`, `readStdinLines`.
- Expressions régulières (D157) : `test`, `search`, `countMatches`, `matches`, `replace`, `replaceAll`. `Map` (D158) : `Map(entrées)`, `countBy`, `size`, `get`, `has`, `keys`, `values`, `entries`, `set`, `delete`.
- Le guide (`docs/language.md`) donne le type et un exemple testé de chacune.

## 7. Règles de fusion

Une fusion évite de créer un tableau intermédiaire en mémoire ; les règles ci-dessous sont garanties, et ce sont les seules sur lesquelles `requireFused` s'appuie.

Préalable : avant la fusion, **toute** fonction non récursive est intégrée à son point d'appel (inlining), sans seuil, et toute fonction d'ordre supérieur est spécialisée (section 11). Un seuil d'inlining rendrait la fusion dépendante de l'implémentation ; la croissance du code qui en résulte est une limite connue (section 16).

### Règles de fusion

| Règle | Forme | Garantie |
| --- | --- | --- |
| F1 verticale | un producteur élément par élément (`map`, opérateurs élément par élément, `zip`, `iota`, `fill`, vues) alimente un seul consommateur (`map`, `reduce`, `scan`, `filter`, `fold`, indexation) | un seul parcours, aucun tableau intermédiaire |
| F2 filter | `filter` alimente un seul `map`, `reduce` ou `fold` | un seul parcours |
| F3 horizontale | plusieurs `reduce` indépendants sur un même tableau, dans une même fonction | un seul parcours, résultats regroupés |
| F4 vues | `window`, `x[a..b]`, `zip` | jamais matérialisés, même partagés, sauf par la règle M3 |

### Règles de matérialisation

Un tableau intermédiaire est stocké en mémoire si et seulement si une de ces règles s'applique.

| Règle | Cas |
| --- | --- |
| M1 | un résultat (hors vues) utilisé plus d'une fois : calculé une fois, stocké, jamais recalculé |
| M2 | le résultat d'un `scan`, d'une fonction récursive ou d'un `fold` qui produit un tableau |
| M3 | une valeur (vues comprises) transmise à une `action` ou retournée par `main` |

### Exemple

`stdDev` (section 3) fait deux parcours de `x`. Le premier calcule `average(x)`. Le second fusionne `(x - m) ** 2` et `sum` par F1 ; aucun tableau n'est créé. Deux parcours sont nécessaires, car `m` doit être connu avant le second.

Chaque matérialisation figure dans le memory report, avec la règle qui l'a causée.

## 8. Modèle mémoire

Aucun ramasse-miettes : chaque allocation est libérée à un point connu à la compilation, ou par comptage de références quand ce n'est pas possible.

### Placement des valeurs

- Scalaires, tuples, records et types somme non récursifs : registres ou pile.
- Tableaux : un bloc contigu, aligné sur 64 octets (une ligne de cache sur les CPU x86-64 courants), sur le tas.
- Le compilateur peut placer sur la pile un tableau de taille constante ; le seuil est à fixer (section 16). Ce choix n'affecte que la vitesse, jamais le verdict d'une exigence (section 9).
- Types récursifs : sur le tas, avec comptage de références ; un constructeur sans charge n'est pas alloué (D169).
- Un littéral de tableau dont tous les éléments sont des constantes scalaires est une donnée du programme : le lire n'alloue rien (`@requireNoAlloc` l'accepte), et une valeur qui en est faite (`with`, valeur initiale d'un `fold`) en est une copie (D173).

### Unicité

Une valeur est **unique** à un point du programme si aucune utilisation ultérieure ne la référence.

1. L'analyse d'unicité porte sur l'IR, après inlining et spécialisation (section 13).
2. Dans `e.with(i, v)`, `i` et `v` sont évalués avant que `e` soit consommé. Une lecture scalaire de `e` dans `i` ou `v` (comme `c[v] + 1`) ne casse pas l'unicité.
3. Si `e` est unique : mise à jour en place, aucune allocation.
4. Sinon : copie de `e`, puis mise à jour. La copie apparaît dans le memory report.
5. `requireUnique(e.with(i, v))` : si `e` n'est pas unique, erreur de compilation citant la ligne de l'autre utilisation.
6. L'accumulateur de `fold` est unique s'il n'est utilisé nulle part ailleurs. Dans `histogram`, le tableau `zeros()` est donc mis à jour en place à chaque étape.
7. Une vue (`window`, `x.slice(a, b)`, `zip`) partage la mémoire de sa source : tant qu'elle est vivante, la source n'est pas unique.

### Algorithme d'unicité (D161)

L'analyse s'applique au corps de chaque fonction de l'IR, après inlining et spécialisation. Elle décide, pour chaque valeur qui possède de la mémoire (tableau, chaîne, `bigint`, nœud d'un type récursif, tuple qui en contient), où elle est déplacée, copiée ou libérée. Les étapes, dans l'ordre :

1. **Régions.** Le corps forme un arbre de régions : la région de la fonction ; chaque branche d'un `if` ouvre une région marquée par ce `if` et son côté ; le corps de la lambda d'un `map`, `reduce`, `fold`, `scan` ou `filter` ouvre une région sans côté (il peut s'exécuter plusieurs fois).
2. **Ordre.** Chaque lecture d'une variable reçoit un numéro dans l'ordre d'évaluation : de gauche à droite et en profondeur, la valeur d'un `const` avant la suite, `i` et `v` avant `e` dans `e.with(i, v)` (règle 2 ci-dessus).
3. **Possession et alias.** Une variable possède sa valeur si l'expression qui la définit produit de la mémoire neuve (un littéral, un `map`, le résultat d'un appel ou d'une action) ; l'accumulateur d'un `fold` possède la sienne, et `main` possède le tableau de ses arguments. Une variable définie par une vue ou une partie d'une autre (une variable, une composante, une ligne, une fenêtre, une tranche qui est une vue) est un alias de ses bases : ses lectures comptent comme des lectures de ses bases, et elle ne possède rien. Un tuple neuf décomposé par `const [a, b] = t` transmet la possession à ses composantes.
4. **Dernière lecture sur son chemin.** Une lecture d'une variable `v` qui possède sa valeur est candidate si sa région est celle où `v` est définie, ou une branche de `if` sous elle atteinte sans traverser de corps de lambda, et si chaque lecture postérieure de `v` (ou d'un de ses alias) est dans l'autre branche d'un `if` qui les sépare. Il y a ainsi au plus une candidate par chemin d'exécution.
5. **Positions qui prennent une valeur.** Prennent la valeur qu'elles reçoivent : la cible et le nouvel élément d'une mise à jour `with`, la valeur initiale d'un `fold`, le résultat d'une fonction, d'un corps de `const`, de `fold` ou de `bench`, une composante d'un tuple ou d'un littéral de tableau, le contenu d'un nœud, l'élément neutre et chaque résultat de l'opérateur d'un `reduce` ou `scan` de valeurs qui possèdent de la mémoire. Toutes les autres positions ne font que lire : indice, longueur, entrées d'un `map`, opérandes des opérations de chaînes, arguments des actions et de `print`.
6. **Déplacement ou copie.** Quand une position qui prend une valeur reçoit une valeur empruntée dont l'expression finit par une variable `v`, la lecture est un déplacement si elle est candidate (étape 4) et si `v` possède sa valeur ; sinon une copie explicite est insérée, et le memory report en donne la raison : `v` est lue de nouveau (à la ligne indiquée), `v` est une entrée ou une partie d'une autre valeur, `v` est définie hors de cette boucle ou de cette branche, ou une partie de `v` est gardée comme une valeur à part. Pour un `if` dont les deux branches empruntent, chaque branche est traitée séparément.
7. **`requireUnique`.** Si l'étape 6 doit copier la cible d'un `with` marqué `requireUnique`, la compilation échoue avec cette raison.
8. **Libération.** Une variable qui possède sa valeur et n'est pas déplacée sur tous les chemins est libérée à la fin de sa portée ; si elle est déplacée dans une branche d'un `if` et pas dans l'autre, elle est libérée au début de l'autre branche. Une valeur neuve qui n'est que lue est d'abord liée à une variable, libérée après la lecture.
9. **Vérification.** Un vérificateur indépendant relit l'IR obtenu (D96) : chaque valeur possédée est libérée une fois ou déplacée, aucune valeur empruntée n'est donnée, aucune variable n'est lue après son déplacement, aucune vue ne survit à sa source. Un échec est un défaut du compilateur, jamais une erreur du programme.

Les nœuds des types récursifs portent un compteur de références (D103) ; les tableaux n'en ont pas : une valeur partagée est copiée, et chaque copie figure dans le memory report.

### Résultats et destination-passing

Quand une fonction produit un nouveau tableau, l'appelant fournit la mémoire du résultat (destination-passing). L'allocation n'est pas supprimée : elle est déplacée chez l'appelant, qui peut la réutiliser d'un appel à l'autre. C'est ce qui rend `requireNoAlloc` utilisable sur des fonctions qui retournent des tableaux.

### Libération

- Une valeur unique est libérée juste après sa dernière utilisation.
- Une valeur partagée dont la durée de vie n'est pas déterminable à la compilation passe en comptage de références. Le comptage est non atomique si la valeur ne quitte pas un thread, atomique sinon ; c'est le compilateur qui crée les threads, il sait donc lequel choisir.
- Aucun cycle n'est possible : dans un langage strict et pur, une valeur ne peut pas se référencer elle-même. Lean repose sur la même propriété (Ullrich & de Moura, 2019).

### Memory report

Produit à chaque compilation avec `--report`, en texte et en JSON.

```
Memory report — analysis.isc
  line 12  copy        8,000 bytes   `a` still used at line 14
  line 20  alloc       [n]f64        shared result (rule M1)
  line 31  refcount    Tree<f64>     recursive type
  line 40  bounds      1 remaining check: x[i + 1]
```

## 9. Exigences de performance

Une exigence est une garantie écrite par le programmeur ; si le compilateur ne peut pas la tenir, la compilation échoue avec l'explication.

| Exigence | Portée | Garantie | Vérifiée sur |
| --- | --- | --- | --- |
| `requireUnique(...)` | une mise à jour `x.with(i, v)` | mise à jour en place, aucune copie | analyse d'unicité (section 8) |
| `@requireNoAlloc` | une `function` et tout ce qu'elle appelle | aucune création de tableau ni allocation, ni sur la pile ni sur le tas, hors la destination fournie par l'appelant (section 8) | IR après fusion et unicité |
| `@requireFused` | une `function` | aucun tableau intermédiaire matérialisé à l'intérieur | règles F1 à F4 et M1 à M3 (section 7) |
| `@requireTail` | une `function` récursive | chaque appel récursif est terminal ; compilé en boucle | IR après inlining |

### Portabilité des verdicts

- Une exigence se vérifie uniquement à partir des règles de cette spécification, jamais des optimisations propres à une implémentation.
- Elle donne donc le même verdict sur toute cible et avec tout compilateur conforme. C'est pourquoi `requireNoAlloc` interdit aussi les tableaux sur la pile : le seuil de placement sur la pile ne peut pas changer le verdict.
- Conséquence : l'algorithme d'analyse d'unicité est spécifié pas à pas (section 8, D161).

### Exemples

```
@requireTail
function gcd(a: u64, b: u64): u64 {
    if (b == 0) {
        return a;
    } else {
        return gcd(b, a % b);
    }
}

@requireFused
function center(x: f64[n]): f64[n] {
    const y = x * 2.0;
    return y - mean(y);
}
```

`gcd` est accepté. `center` est refusé :

```
error: requireFused not satisfied (line 11)
  `y` (line 12) is used twice (line 13): it is materialized (rule M1)
```

## 10. Arithmétique flottante

Par défaut, un calcul flottant donne le même résultat au bit près, quels que soient la cible et le nombre de cœurs ; pour les NaN, la garantie porte sur la valeur observable, définie ci-dessous.

### Règles de base

- IEEE 754-2019, binary32 et binary64, arrondi au plus proche pair.
- Aucune contraction implicite en FMA : `a * b + c` fait deux arrondis. Le FMA s'écrit explicitement : `fma(a, b, c)`.
- Les nombres sous-normaux sont conservés.
- Aucune réassociation, aucune simplification algébrique : `x * 0.0` n'est pas remplacé par `0.0`, à cause de NaN et de −0.
- `sqrt` est correctement arrondie, comme l'exige IEEE 754. Les fonctions transcendantes (`pow`, `exp`, `log`, `sin`, `cos`) sont fournies par le runtime d'Isocron, avec la même implémentation sur toutes les cibles ; leur précision cible est en section 16.

### NaN

- Les processeurs ne produisent pas les mêmes bits de NaN : quand une opération crée un NaN sans opérande NaN, x86 met le bit de signe à 1 et ARM à 0 ([WebAssembly design, issue 619](https://www.github.com/WebAssembly/design/issues/619)).
- Règle : en v0, aucune opération du langage ne permet de lire les bits d'un flottant, et `print` affiche `NaN` sans signe. Tout flottant converti en octets (`writeF64`, `writeF32`) voit ses NaN remplacés par le NaN canonique : signe 0, bit « quiet » à 1, charge utile nulle.
- Conséquence : les bits des NaN ne sont jamais observables, et la garantie « au bit près » tient sans coût dans les calculs.

### `min` et `max`

Décision : `min` et `max` sur flottants suivent les opérations `minimum` et `maximum` d'IEEE 754-2019 : un opérande NaN donne NaN, et −0 est considéré comme inférieur à +0. Le résultat ne dépend donc pas de l'instruction choisie par le processeur.

### Puissance `**`

- `x ** k` avec `k` entier (littéral ou variable entière, `k >= 0`) : calculé par exponentiation binaire, en parcourant les bits de `k` du plus fort au plus faible (élever au carré, puis multiplier par `x` si le bit vaut 1). `x ** 2` donne donc exactement `x * x`. Un `k` négatif est une panne.
- `x ** y` avec `y` flottant : équivaut à `pow(x, y)`, fonction du runtime.

### Forme de réduction fixe

La sémantique de `reduce(x, op, e)` est la forme ci-dessous, et non « un ordre quelconque ». Le résultat est donc déterministe, même si `op` n'est pas associatif.

Paramètres fixés par la spécification : V = 32 voies, B = 4 096 éléments par bloc (multiple de V). V a été choisi après mesures (`results/xeon-lanes.md`, DECISIONS D55 et D56) ; B reste provisoire jusqu'à ses propres mesures (section 16).

```
reduce_block(x[0..m]):                  -- m <= B
    acc[j] = e                 for j = 0 .. V-1
    for i = 0 .. m-1:
        acc[i mod V] = op(acc[i mod V], x[i])
    for d = V/2, V/4, ..., 1:
        acc[j] = op(acc[j], acc[j + d])    for j = 0 .. d-1
    return acc[0]

reduce(x[0..n]):
    if n <= B: return reduce_block(x)
    else:
        split x into consecutive blocks of B elements (the last one may be shorter)
        return reduce([reduce_block(b) for each block b])
```

Pourquoi cette forme :

- **Vectorisable** : les 32 accumulateurs entrelacés correspondent à des registres SIMD. 32 `f64` remplissent quatre registres AVX-512 ou huit AVX2 : assez de chaînes de calcul indépendantes pour masquer la latence de l'addition flottante, ce que 16 accumulateurs ne permettent pas sur ces processeurs. Sur une cible qui a peu de registres (SSE : 16 registres de 2 `f64`), le compilateur met à jour les accumulateurs en plusieurs passes sur le bloc ; chaque accumulateur reçoit toujours ses éléments dans l'ordre, donc le résultat ne change pas.
- **Parallélisable sans changer le résultat** : les blocs sont indépendants, chaque cœur traite des blocs entiers. Le résultat ne dépend pas du nombre de cœurs.
- **Face à du C écrit simplement** : par défaut, GCC n'autorise pas la réassociation des opérations flottantes (`-fassociative-math` est désactivée, [documentation GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html)). Une somme écrite comme une simple boucle doit donc garder l'ordre séquentiel. Nous en déduisons qu'elle n'est pas vectorisée ; c'est à vérifier dans l'assembleur pendant le POC.
- **Limite honnête** : du C écrit à la main avec plusieurs accumulateurs obtient la même vectorisation. L'avantage d'Isocron est de ne pas avoir à l'écrire, pas un gain de vitesse impossible à obtenir en C.

### `scan` flottant

Un `scan` sur des flottants a une forme fixe (D112) : le premier bloc de B éléments est cumulé de gauche à droite depuis l'élément neutre ; chaque bloc suivant est cumulé depuis son propre premier élément, puis chacune de ses valeurs est combinée à la dernière valeur du bloc précédent. Jusqu'à B éléments, c'est le cumul de gauche à droite ; au-delà, il en diffère par l'arrondi des retenues, et il donne les mêmes valeurs pour tout nombre de threads.

### Mode `allowReassoc`

- Décorateur `@allowReassoc` d'une `function` : le compilateur peut réassocier, contracter en FMA et choisir librement la forme de réduction.
- Le résultat peut alors varier selon la cible et le nombre de cœurs. Le memory report liste les fonctions concernées.
- Il n'équivaut pas à `-ffast-math`, qui active aussi d'autres options, dont `-ffinite-math-only`. La comparaison équitable se fait avec `-fassociative-math`, qui exige `-fno-signed-zeros` et `-fno-trapping-math` (section 14).

## 11. Fonctions d'ordre supérieur et récursion

Les fonctions passées en argument sont résolues à la compilation : aucune fermeture (closure) n'est allouée et aucun appel indirect n'est émis.

### Fonctions comme valeurs (v0)

- Une fonction peut être passée en argument, retournée par une fonction non récursive (développée en ligne, D152), ou liée par `const`.
- Elle peut être rangée dans un tuple, un record ou un tableau, connus à la compilation (D114), et dans un champ d'un type somme : la valeur y tient le code de la fonction parmi celles que le programme range dans ce champ, et l'appel est un aiguillage sur ce code (D153). Une fonction rangée dans un type somme ne lit ni variable ni taille de son contexte.
- Une lambda peut capturer des variables : elles deviennent des arguments supplémentaires après spécialisation (lambda lifting).
- Chaque appel d'une fonction d'ordre supérieur est spécialisé pour la fonction reçue : `map(x, f)` devient une boucle dédiée à `f`.
- Si la fonction dépend d'une condition connue seulement à l'exécution (`const f = c ? g : h;`), elle est compilée en branchement : chaque fonction possible reçoit une étiquette, et l'appel devient un aiguillage sur cette étiquette (defunctionalization, Reynolds 1972). Aucune allocation ; le cas est signalé dans le memory report.

### Récursion

- Récursion terminale directe : doit être compilée en boucle, sans croissance de pile.
- Récursion terminale mutuelle (D113) : un appel en position terminale d'une fonction récursive à une autre réutilise le cadre quand les tableaux qu'il passe sont des paramètres de l'appelant et que la boucle de l'appelant ne possède aucun tableau ; `@requireTail` l'accepte.
- Récursion non terminale : autorisée, utilise la pile, signalée dans le memory report. Un dépassement de pile est une panne.
- Une `function` récursive n'est pas inlinée ; son résultat tableau est matérialisé (règle M2).

## 12. Effets et entrées/sorties

Les effets n'existent que dans les `action`, qui forment la couche extérieure du programme ; le cœur reste pur.

- Une `action` peut appeler des `function` et d'autres `action`. Une `function` qui appelle une `action` est une erreur de typage.
- Point d'entrée : `action main(args: string[k]): i32`. Le résultat est le code de sortie du programme.
- `declare function` déclare une fonction C (ABI System V x86-64), appelable uniquement depuis une `action`.
- En v0, une erreur d'entrée/sortie est une panne.

### Ordre d'exécution dans une `action`

Les instructions d'une `action` s'exécutent dans l'ordre écrit, **calculs purs compris** : la valeur d'un `const` est entièrement calculée avant l'instruction suivante. Le compilateur ne peut ni la déplacer au-delà d'une autre instruction, ni la supprimer, même si elle n'est pas utilisée ensuite. Sans cette règle, un calcul pur pourrait légalement sortir de la zone chronométrée (section 5, ordre libre).

### Actions de base (v0)

| Action | Type | Rôle |
| --- | --- | --- |
| `readBytes(path)` | `(string) => u8[k]` | lit un fichier entier |
| `writeBytes(path, x)` | `(string, u8[n]) => void` | écrit un fichier |
| `readF64(path)` | `(string) => f64[k]` | lit un fichier binaire de flottants 64 bits, petit-boutiste |
| `readF32(path)` | `(string) => f32[k]` | idem en 32 bits |
| `writeF64(path, x)`, `writeF32` | `(string, f64[n]) => void`, `(string, f32[n]) => void` | écrit un fichier binaire petit-boutiste ; NaN canoniques (section 10) |
| `print(v)` | `(T) => void` | écrit `v` sur la sortie standard ; un flottant est écrit sous la forme décimale la plus courte qui se relit en la même valeur (décision) |
| `clock()` | `() => u64` | horloge monotone, en nanosecondes |
| `bench(r, f)` | `(i64, () => U) => [U, u64]` | appelle `f` `r` fois et retourne le dernier résultat et la durée totale en nanosecondes |

### Règle de `bench`

Chacun des `r` appels de `f` doit être évalué entièrement. Le compilateur ne peut pas les fusionner, les factoriser en un seul calcul, ni en supprimer, même si `f` est pure et reçoit les mêmes données. C'est la primitive de mesure du protocole de la section 14.

```
action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const [r, ns] = bench(1000, () => sum(x));
    print(ns);
    writeF64(args[1], [r]);
    return 0;
}
```

## 13. Architecture du compilateur

Le compilateur applique toutes les garanties de la spécification sur sa propre IR ; LLVM ne sert qu'à la génération du code machine.

&#91;embedded content: chaîne de compilation d'Isocron · 11 étapes, 3 sorties annexes\]

Les règles de fusion, l'unicité et les exigences sont décidées au middle-end, sur des règles écrites ici. Un échec d'exigence arrête la compilation avant LLVM.

### Choix techniques

Pour le POC, je recommande LLVM directement plutôt que MLIR : moins de couches, et la fusion se fait de toute façon dans l'IR propre à Isocron.

| Composant | Choix recommandé | Raison |
| --- | --- | --- |
| Langage du compilateur | Rust | accès mature à LLVM, sûreté mémoire, langage que tu utilises déjà |
| Lexer et parser | écrits à la main, descente récursive | indentation significative, messages d'erreur précis |
| IR haut niveau | IR propre à Isocron | fusion, unicité et exigences doivent suivre cette spécification, pas un outil externe |
| Back-end | LLVM via [inkwell](https://github.com/TheDan64/inkwell) | génération de code éprouvée pour x86-64 et ARM64 |
| Flottants dans LLVM | opérations émises sans drapeaux fast-math | sans les drapeaux `reassoc` et `contract`, LLVM ne réassocie ni ne contracte ([LangRef](https://llvm.org/docs/LangRef.html)) |
| MLIR | reporté | plus puissant mais plus complexe ; utile surtout pour un futur back-end GPU |
| Runtime | minimal : allocation alignée, comptage de références, pool de threads, fonctions mathématiques | aucun coût caché à l'exécution |
| Interpréteur | sur l'IR haut niveau, simple et lent | oracle bit à bit pour tous les tests |

## 14. Protocole de benchmark

Isocron « bat » un langage sur un noyau si sa médiane est plus basse que la meilleure médiane de ce langage, toutes variantes confondues, d'un écart supérieur à la dispersion mesurée ; l'objectif est de battre ainsi C et Rust.

### Langages et options

| Langage | Variante stricte (comparée au mode par défaut d'Isocron) | Variante réassociée (comparée à `allowReassoc`) | Parallélisme |
| --- | --- | --- | --- |
| C (GCC et Clang) | `-O3 -march=native -ffp-contract=off` | `-O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math` | OpenMP |
| Rust | `cargo build --release`, `-C target-cpu=native` | — | rayon |
| Go | `go build`, `GOAMD64=v3` | — | goroutines |
| Zig | `-O ReleaseFast`, `-mcpu=native` | `@setFloatMode(.optimized)` | `std.Thread` |
| C++, Fortran (optionnels) | `g++` / `gfortran`, `-O3 -march=native` | — | OpenMP |

- **FMA** : selon la documentation GCC, le défaut est `-ffp-contract=off` en mode C conforme (`-std=c11` ou similaire), et `-ffp-contract=fast` sinon. Sans `-ffp-contract=off`, GCC peut donc utiliser des FMA qu'Isocron s'interdit. Pour les autres langages, vérifier et documenter si le compilateur contracte en FMA par défaut.
- `-ffast-math` n'est pas utilisé : il active aussi `-ffinite-math-only` et d'autres options sans équivalent dans `allowReassoc`.

### Noyaux

| Noyau | Ce qu'il teste | POC |
| --- | --- | --- |
| Somme `f64` | réduction à forme fixe | oui |
| Produit scalaire `f64` | fusion `map` + `reduce` (F1) | oui |
| Histogramme `u8` | mise à jour en place dans un `fold` | oui |
| Moyenne puis écart type `f64` | deux parcours, fusion, `** 2` | oui |
| Moyenne glissante, fenêtre 64 | vues, `window` | non |
| Filtre puis somme | fusion F2 | non |
| Cumul (scan) `i64` | scan parallèle entier | non |
| Produit matrice-vecteur `f64`, 4 096 × 4 096 | tableaux 2D | non |

Deux tailles par noyau :

- **Petite**, tenant dans le cache L2 de la machine de test (à dimensionner une fois la machine connue). Elle mesure la qualité du code généré : c'est elle qui compte pour le point de décision du POC.
- **Grande**, 10⁷ éléments. Somme et produit scalaire y sont limités par la bande passante mémoire ; toute implémentation vectorisée plafonne au même niveau. Elle sert à vérifier le passage à l'échelle, pas à départager.

### Deux comparaisons

- **A. Même algorithme** : tous les langages implémentent le même algorithme. Elle compare les compilateurs.
- **B. Meilleur algorithme par langage** : chaque langage utilise ce qu'un développeur expérimenté écrirait (par exemple une moyenne glissante en O(n) en C, au lieu de `window` + `sum` en O(n·k)). Elle compare ce que chaque langage permet d'écrire. Les résultats flottants peuvent alors différer : l'écart est mesuré et documenté.
- Dans les deux cas, chaque langage concurrent a **toujours** une version idiomatique et une version optimisée à la main (accumulateurs multiples, SIMD explicite). Isocron est comparé à la meilleure des deux.

### Mesure

- Une machine fixe, documentée : CPU, RAM, OS, versions de chaque compilateur.
- Fréquence CPU fixée (gouverneur `performance`, turbo désactivé si possible), processus épinglé sur des cœurs.
- Seul le noyau est chronométré, chargement des données exclu. Dans Isocron, la mesure passe par `bench` (section 12).
- Le noyau est répété `r` fois par mesure, `r` étant choisi pour que chaque mesure dure au moins 100 ms (décision). Dans les autres langages, chaque résultat est protégé contre l'élimination par l'outil prévu (par exemple `std::hint::black_box` en Rust).
- 5 mesures de chauffe, puis 30 mesures ; on rapporte la médiane et l'écart interquartile du temps par appel.
- Les résultats sont écrits en binaire (`writeF64`) et comparés : en mode reproductible, Isocron doit être identique au bit près à son interpréteur de référence.

## 15. Feuille de route

On construit d'abord un POC minimal, on le mesure contre C, Rust, Go et Zig sur 4 noyaux, et on ne poursuit le langage complet que si les résultats le justifient.

### Étape 1 — POC

Sous-ensemble du langage implémenté :

- Types : `i64`, `u64`, `f64`, `u8`, `u32`, `boolean`, `string`, tuples, tableaux à une dimension `T[n]`, variables de taille et tailles existentielles.
- Expressions : `function`, `const` (avec motifs de tuple), `if`/`else` et `? :`, opérateurs dont `**` à exposant entier, lambdas avec capture, conversions `f64(…)`, littéraux de tableau `[…]`.
- Primitives : `map`, `reduce`, `fold`, opérateurs élément par élément, `x[i]`, `x.with(i, v)`, `zeros`, et dans la bibliothèque standard `sum`, `mean`, `dot`, `sqrt`.
- Garanties : fusion F1 seulement ; forme de réduction de la section 10, vectorisée ; mise à jour en place quand l'unicité est évidente (accumulateur de `fold`).
- Effets : `action main`, `readF64`, `readBytes`, `writeF64`, `print`, `bench`, `parseI64`, avec l'ordre d'exécution de la section 12.
- Monothread uniquement.

### Programmes du POC

Ces 4 programmes sont la référence des tâches P1 à P6. Chacun lit ses données, chronomètre le noyau avec `bench`, affiche la durée et écrit le résultat en binaire.

```
// sum.isc        usage: sum <input.f64> <output.f64> <reps>
action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const [r, ns] = bench(parseI64(args[2]), () => sum(x));
    print(ns);
    writeF64(args[1], [r]);
    return 0;
}
```

```
// dot.isc        usage: dot <a.f64> <b.f64> <output.f64> <reps>
action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = readF64(args[1]);
    const [r, ns] = bench(parseI64(args[3]), () => dot(a, b));
    print(ns);
    writeF64(args[2], [r]);
    return 0;
}
```

```
// histogram.isc  usage: histogram <input.u8> <output.f64> <reps>
function histogram(values: u8[n]): u32[256] {
    return fold(values, zeros(), (c, v) => requireUnique(c.with(v, c[v] + 1)));
}

action main(args: string[k]): i32 {
    const x = readBytes(args[0]);
    const [h, ns] = bench(parseI64(args[2]), () => histogram(x));
    print(ns);
    writeF64(args[1], map(h, (c) => f64(c)));
    return 0;
}
```

```
// std_dev.isc    usage: std_dev <input.f64> <output.f64> <reps>
function stdDev(x: f64[n]): f64 {
    const m = mean(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const [r, ns] = bench(parseI64(args[2]), () => stdDev(x));
    print(ns);
    writeF64(args[1], [r]);
    return 0;
}
```

Dans `dot`, `a` et `b` ont des tailles existentielles différentes : l'égalité de leurs tailles est testée à l'entrée de `dot` (section 4).

### Tâches du POC

| Tâche | Livrable | Critère d'acceptation |
| --- | --- | --- |
| P0 Fondations | workspace Rust : crates `isc-syntax`, `isc-types`, `isc-ir`, `isc-interp`, `isc-codegen`, `isc-runtime`, binaire `iscc` | `cargo test` passe |
| P1 Front-end | lexer avec indentation, parser, AST, erreurs avec position | les 4 programmes ci-dessus sont analysés |
| P2 Typage | sous-ensemble ci-dessus, typage des littéraux, tailles vérifiées | les 4 programmes sont typés ; erreurs de type et de taille détectées sur des cas négatifs |
| P3 Interpréteur | interprète l'IR avec la forme de réduction fixe | sert d'oracle bit à bit |
| P4 Codegen LLVM | IR vers LLVM IR, fusion F1, réduction vectorisée, mise à jour en place, `bench` | binaires identiques au bit près à l'interpréteur ; SIMD visible dans l'assembleur |
| P5 Programmes de référence | les 4 noyaux en C, Rust, Go, Zig, en version idiomatique et optimisée à la main | résultats identiques, ou écart flottant documenté |
| P6 Mesures | scripts du protocole de la section 14, tableau des médianes | rapport de résultats publié dans le dépôt |

**Point de décision** : sur la petite taille (cache L2), si Isocron n'égale pas au moins la meilleure variante stricte de C et de Rust sur la somme et le produit scalaire, on analyse l'assembleur généré avant d'aller plus loin.

### Étape 2 — Langage complet

| Phase | Livrable | Critère d'acceptation |
| --- | --- | --- |
| 1 | records (en colonnes), types somme, génériques, classes `Num/Ord/Eq`, paramètres `size` | exemples des sections 3 et 4 typés |
| 2 | fusion F2 à F4, règles M1 à M3, memory report | aucun tableau intermédiaire hors règles M |
| 3 | analyse d'unicité complète, comptage de références de secours, `requireUnique` | histogramme sans allocation ; aucune fuite (Valgrind ou AddressSanitizer) |
| 4 | `requireNoAlloc`, `requireFused`, `requireTail` | tests positifs et négatifs pour chacune |
| 5 | `filter`, `scan`, `window`, `zip`, tranches, `min`/`max` IEEE 754-2019, NaN canoniques | sémantique conforme aux sections 6 et 10 |
| 6 | parallélisme par blocs, pool de threads | résultats identiques avec 1, 2, 4… cœurs |
| 7 | suite complète de benchmarks (section 14), comparaisons A et B | tableau médiane et IQR contre tous les langages |

### Consignes d'implémentation

- Cette spécification est la source de vérité. Toute ambiguïté rencontrée est ajoutée à la section 16, jamais tranchée en silence dans le code.
- Tests d'abord : chaque règle (fusion, unicité, réduction) a ses tests positifs et négatifs.
- Test différentiel : générer des programmes aléatoires bien typés et comparer compilateur et interpréteur au bit près.
- Ne jamais activer de drapeau fast-math dans LLVM hors `allowReassoc`.
- Tout le code, les identifiants, les messages et la documentation du dépôt sont en anglais.

## 16. Questions ouvertes et limites connues

Ces points ne sont pas tranchés ; aucune implémentation ne doit les décider sans mise à jour de ce document.

| Question | Pourquoi elle compte | À trancher avant |
| --- | --- | --- |
| Valeur de B (4 096) ; V = 32 est tranché (section 10) | grain du parallélisme et coût des niveaux supérieurs de l'arbre, pas encore mesurés | étape 2 |
| Coût réel du mode reproductible face aux variantes réassociées | conditionne l'intérêt du mode par défaut | fin du POC |
| `@requireVectorized` et cibles multiples | une même fonction peut passer sur une cible et échouer sur une autre | après la phase 7 |
| Débordement entier modulaire | peut bloquer des optimisations que C obtient par comportement indéfini | après mesures |
| Croissance du code due à l'inlining total | l'inlining sans seuil garantit la fusion mais peut gonfler les binaires ; toute limite devra être une règle de la spécification, pas une heuristique | étape 2, phase 2 |
| Seuil de placement sur la pile | performances des petits tableaux (n'affecte aucun verdict) | étape 2, phase 3 |
| Précision des fonctions transcendantes | reproductibilité contre vitesse | étape 2 |
| Disposition automatique (lignes ou colonnes) | aujourd'hui toujours en colonnes | après la phase 7 |
| Comportement FMA par défaut de Rust, Go, Zig, Fortran | équité de la variante stricte | P5 du POC |
| Unicité du nom « Isocron » | deux recherches web n'ont trouvé aucun langage de ce nom ; pas une preuve | avant publication |
| Valeurs linéaires (`Region`, `Shared`, blocs) : l'unicité de la section 8 en garantie, et non plus en optimisation | sans elle, le runtime en Isocron (section 17) pourrait utiliser une région rendue | avant l'allocateur en Isocron |
| Moteur des expressions régulières et grands entiers : réécrits en Isocron ou gardés en Rust | des dizaines de milliers de lignes, très optimisées | avant la fin de la section 17 |
| Accès aux instructions vectorielles explicites (masques, comparaisons d'octets) depuis les modules du runtime | les recherches du runtime en dépendent pour leur vitesse | avant le portage des fonctions de texte |

### Questions tranchées pour la v1 (D161)

- **Algorithme d'unicité** : écrit pas à pas à la section 8 ; c'est celui de l'implémentation, qu'un vérificateur indépendant contrôle.
- **Passage des valeurs aux fonctions `extern`** (section 12, D125) : un tableau de scalaires en pointeur suivi de sa longueur `int64_t`, une `string` en copie terminée par un octet nul (une chaîne qui contient un octet nul est une panne), au plus 6 paramètres entiers et 8 flottants, pas de fonction variadique, un résultat scalaire ou `void`.
- **Partie 3** (bibliothèque et syntaxe des programmes idiomatiques), dans la grammaire de la section 3 : chaînes (littéraux, `+`, ordre par octets, D134), texte et CSV (D135), opérateurs bit à bit aux niveaux de TypeScript (décalage modulo le nombre de bits), `concat` des tableaux, `printBytes` (D136), `Result` et actions `try` (D138), modules et imports nommés, `export` (D139, D150), gabarits (D150), noms de TypeScript (D151), fonctions de `Math`, de chaînes, `toFixed`, entrée standard, `random`, `sort` (D155), `bigint` et analyse des nombres (D156), expressions régulières en temps linéaire (D157), `Map<K, V>`, `Option` et `??` (D158), alias de types et constantes (D159). Les dix programmes du Benchmarks Game s'écrivent avec (D160).
- **Fonctions stockées dans des structures** : dans les tuples, records, tableaux (D148) et types somme (D153), chaque appel spécialisé pour chaque fonction possible ; une fonction stockée dans un type somme qui lit une variable de son contexte reste refusée.
- **Récursion terminale mutuelle** (D113) : un appel terminal à une autre fonction récursive réutilise le cadre quand les tableaux passés survivent à l'appelant.
- **`scan` flottant parallèle à forme fixe** (D112) : le premier bloc de B éléments cumulé depuis l'élément neutre, chaque bloc suivant depuis son premier élément puis combiné à la retenue du bloc précédent ; les mêmes valeurs pour tout nombre de threads.
- **`scatter` et règle de collision** (D111) : les valeurs d'un même indice combinées dans l'ordre de k, quel que soit l'opérateur.

## 17. Le runtime écrit en Isocron

Le runtime des programmes compilés (allocateur, pool de threads, démarrage, entrées et sorties, fonctions de chaînes et de texte) est aujourd'hui écrit en Rust, et une partie de ce code n'est pas vérifiée par le compilateur Rust (`unsafe`). But : le runtime écrit en Isocron, vérifié comme tout programme (types, bornes, unicité), au-dessus d'une liste fermée de **primitives du système** fournies par le compilateur, seule base de confiance. C'est aussi le chemin vers un `iscc` construit par `iscc` (D290).

### Principes

1. Les modules du runtime sont livrés avec le compilateur (`isocron:sys`, `isocron:heap`, `isocron:pool`, `isocron:text`). Seuls ces modules peuvent importer `isocron:sys` ; un programme qui l'importe est refusé à la compilation.
2. Les primitives sont énumérées ci-dessous, sans exception implicite. Chacune a un contrat écrit, une implémentation dans le compilateur (et dans l'interpréteur, pour le même résultat partout) et ses tests.
3. Une primitive ne reçoit jamais d'adresse : un tableau ou une région, dont le compilateur passe lui-même l'adresse et la taille. Le noyau ne peut donc ni lire ni écrire hors de ce qui lui est donné.
4. La mémoire brute est une valeur **unique** (section 8) : consommée exactement une fois, le vérificateur de l'IR (D96) le contrôle. Une région rendue au système ne peut plus être lue.
5. L'état partagé entre threads n'existe que dans une région partagée, dont chaque accès est atomique : aucune course de données n'est possible, même dans le runtime.

### Primitives

| Famille | Primitives | Contrat |
| --- | --- | --- |
| Appels système | `sysRead(fd, buf: u8[n])`, `sysWrite(fd, data: u8[n])`, `sysOpen(path, flags, mode)`, `sysClose(fd)`, `sysSeek`, `sysExit(code)`, `sysClock(id)` | les numéros et sens de Linux ; un tableau reçu est la seule mémoire que le noyau lit ou écrit ; le résultat est le code du noyau (négatif en cas d'erreur) ; `buf` est consommé et rendu (mis à jour en place, section 8) |
| Régions | `regionMap(bytes) => Region`, `regionUnmap(r)`, `regionAdvise(r, advice)` ; `load64(r, at)`, `store64(r, at, v)` et leurs variantes de 1, 2, 4 octets, bornés | une `Region` est unique ; `regionUnmap` la consomme ; chaque lecture et écriture est vérifiée contre sa taille |
| Blocs | `arrayOf<T>(r, at, count) => T[count]`, `blockOf<T>(x: T[n]) => [Region, i64]` | les deux seules primitives qui relient la mémoire brute aux tableaux du langage : un bloc d'une région devient la mémoire d'un tableau, et un tableau libéré redevient un bloc ; c'est l'allocateur qui les appelle, et elles seules |
| Mémoire partagée | `sharedMap(bytes) => Shared`, `atomicLoad`, `atomicStore`, `atomicAdd`, `atomicCas`, `wait(s, at, expected)`, `wake(s, at, count)` | une `Shared` n'a que des accès atomiques ; `wait` et `wake` sont le futex de Linux |
| Threads | `spawn(stackBytes, task: action)`, `join(t)`, `threadLocal() => Region` | utilisés par `isocron:pool` seul ; la région locale d'un thread n'est lue que par lui |
| Démarrage | `isc_start` (assembleur du compilateur) | les relocalisations, le stockage local du thread principal, puis le `main` du runtime en Isocron, qui appelle celui du programme |

### Ordre de travail

1. Les primitives dans le compilateur et l'interpréteur, avec leurs tests.
2. Les fonctions pures du runtime (découpe en lignes, `split`, `replace`, tri, analyse et écriture des nombres) en modules Isocron, une à une. Chacune garde sa version Rust tant que la version Isocron est plus lente, mesuré sur les dix programmes.
3. L'allocateur (`isocron:heap`) sur les régions et les blocs, ses métadonnées dans ses régions, ses caches dans les régions locales des threads.
4. Le pool (`isocron:pool`) sur `Shared`, `wait`, `wake` et `spawn`.
5. Les entrées et sorties (fichiers, entrée standard projetée).
6. Restent hors d'Isocron, à décider (section 16) : le moteur des expressions régulières (aujourd'hui `regex-automata`, en Rust) et les grands entiers (`num-bigint`).

## 18. Références

- Blelloch, G. E. *Prefix Sums and Their Applications*. Rapport technique, Carnegie Mellon University, 1990.
- Blelloch, G. E. *Programming Parallel Algorithms*. Communications of the ACM, 1996.
- Henriksen, T. et al. *Futhark: Purely Functional GPU-Programming with Nested Parallelism and In-Place Array Updates*. PLDI 2017. [futhark-lang.org](https://futhark-lang.org)
- Ullrich, S. & de Moura, L. *Counting Immutable Beans: Reference Counting Optimized for Purely Functional Programming*. IFL 2019. [arXiv:1908.05647](https://arxiv.org/pdf/1908.05647v2.pdf). Les auteurs y précisent qu'en Lean il est impossible de créer des structures cycliques.
- Reinking, A., Xie, N., de Moura, L. & Leijen, D. *Perceus: Garbage Free Reference Counting with Reuse*. PLDI 2021. [Koka](https://koka-lang.github.io)
- Tofte, M. & Talpin, J.-P. *Region-Based Memory Management*. Information and Computation, 1997.
- Bernardy, J.-P. et al. *Linear Haskell: Practical Linearity in a Higher-Order Polymorphic Language*. POPL 2018.
- Reynolds, J. C. *Definitional Interpreters for Higher-Order Programming Languages*. ACM Annual Conference, 1972.
- Goldberg, D. *What Every Computer Scientist Should Know About Floating-Point Arithmetic*. ACM Computing Surveys, 1991.
- Hudak, P., Hughes, J., Peyton Jones, S. & Wadler, P. *A History of Haskell: Being Lazy with Class*. HOPL III, 2007.
- IEEE. *IEEE Standard for Floating-Point Arithmetic*, IEEE 754-2019.
- GCC. [Options That Control Optimization](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html) : `-ffp-contract`, `-fassociative-math`, `-ffast-math`.
- WebAssembly design. [Issue 619, bits des NaN](https://www.github.com/WebAssembly/design/issues/619).
- LLVM. [LLVM Language Reference Manual](https://llvm.org/docs/LangRef.html) ; bibliothèque Rust [inkwell](https://github.com/TheDan64/inkwell).
