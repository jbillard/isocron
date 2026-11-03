# The Isocron language — developer guide

This guide describes the language **as the current compiler (`iscc`) implements
it**: the proof of concept of SPEC section 15, extended by the improvements
recorded in `DECISIONS.md`.

> [!NOTE]
> `SPEC.md` (in French) describes the complete design; much of it is not
> implemented yet (see [Not implemented yet](#not-implemented-yet)).

> [!TIP]
> Every example of this guide is compiled and run by the test
> `crates/iscc/tests/docs.rs`, with the interpreter and with the compiler,
> and its output is compared with the output printed here: they all work as
> written.

Isocron is a purely functional, strict array language. Its goal is raw CPU
speed with results that are identical to the bit whatever the machine and the
number of cores: floating-point reductions follow one fixed order, written in
the specification, that the compiler vectorizes and parallelizes without
changing the result.

## Contents

1. [A first program](#a-first-program)
2. [Program structure](#program-structure)
3. [Types](#types)
4. [Expressions](#expressions)
5. [Arrays](#arrays)
6. [Reductions and their fixed order](#reductions-and-their-fixed-order)
7. [Functions as arguments and generics](#functions-as-arguments-and-generics)
8. [Actions and input/output](#actions-and-inputoutput)
9. [Errors and panics](#errors-and-panics)
10. [Performance model](#performance-model)
11. [Not implemented yet](#not-implemented-yet)
12. [The `iscc` command](#the-iscc-command)

## A first program

```isc
// mean of the squares of 0, 0.5, 1, ..., 4.5
function meanSquare(x: f64[n]): f64 {
    return sum(x * x) / f64(n);
}

action main(args: string[k]): i32 {
    const x = map(iota(10), (i) => f64(i) / 2.0);
    print(meanSquare(x));
    return 0;
}
```

```output
7.125
```

Run it with the reference interpreter, or compile it to a native executable:

```sh
target/release/iscc run   first.isc
target/release/iscc build first.isc -o first && ./first
```

The syntax is that of TypeScript; what differs comes from the semantics:

* `function` declares a pure function: it computes a value from its
  arguments, it cannot do any input/output, and it is always inlined where
  it is called.
* `action` declares a function that may do input/output; its body is a
  sequence of statements run in order.
* Every program has exactly one `action main(args: string[k]): i32`. `args`
  are the command-line arguments (without the program name); the returned
  `i32` is the exit code of the process.
* `f64[n]` is an array of `n` doubles. `n` is a *size variable*: it is bound
  by the argument and can be used in the body as an `i64` (`f64(n)` above).
* `const` names a value; nothing can change it, and there is no `let` or
  assignment.

## Program structure

### Comments and layout

`//` starts a comment that runs to the end of the line; `/* ... */` may span
several lines. Blocks are delimited by `{ }` and statements end with `;`;
line breaks and indentation do not matter.

The body of a `function` is a sequence of `const` statements and `if`
statements that ends with `return` on every path. An `if` with a `return`
in one branch decides the value: the rest of the body continues the other
branch, so a test at the top of a function is a guard. `cond ? a : b` is the
conditional expression.

```isc
function poly(x: f64): f64 {
    const a = 2.0 * x;
    const b = a * a + 1.0;
    if (b > 10.0) return b; // a guard
    return -b;
}

action main(args: string[k]): i32 {
    print(poly(1.0));
    print(poly(3.0));
    return 0;
}
```

```output
-5
37
```

### Declarations

```
function name(param: Type, ...): Type { ... }
function name<T extends Num, U>(param: Type, ...): Type { ... } // generic
action name(param: Type, ...): Type { ... }
type Name = A | B(Type, ...);              // a sum type (see "Sum types")
type Name<T> = Type;                       // an alias
const NAME = literal;                      // a constant
const NAME: Type = value;
```

As in TypeScript, `type Name = Type;` gives a name to a type that is not a
sum: a record, a tuple, an array, a function type, `Map<K, V>`, a scalar.
The alias is the type itself, written out wherever its name is used; a size
variable in it (`type Row = f64[n];`) becomes a size variable of the
signature that uses it. A `const` of the top level is a value that any
function can read; without a type it is a literal (or an array of literals)
whose type is that of TypeScript's `number`, `i64` or `f64` (`const N = 3;`,
`const DT = 0.01;`, `const NAMES = ["a", "b"];`), otherwise it takes one
(`const ROOT2: f64 = Math.sqrt(2.0);`). A local variable of the same name
hides it; a constant that depends on itself is an error. Both are exported
and imported as functions are.

```isc
type Point = { x: f64, y: f64 };
type Pair<T> = [T, T];

const ORIGIN: Point = { x: 0.0, y: 0.0 };
const STEPS = 3;

function swap<T>(p: Pair<T>): Pair<T> {
    const [a, b] = p;
    return [b, a] as const;
}

function walk(p: Point, n: i64): Point {
    return n == 0 ? p : walk({ x: p.x + 1.0, y: p.y * 2.0 + 1.0 }, n - 1);
}

action main(args: string[k]): i32 {
    print(walk(ORIGIN, STEPS));
    print(swap([1, 2] as const));
    return 0;
}
```

```output
{x: 3, y: 7}
(2, 1)
```

Functions can call each other in any order. A call of a function that is not
recursive is inlined, which is what guarantees fusion (section 7 of the
specification). A recursive function (one that can call itself, directly or
through others) is compiled as a function of its own: see
[Recursion](#recursion). A user declaration with the name of a
standard-library function (`sum`, `mean`, ...) replaces it everywhere,
including inside the standard library.

### Modules

A file can use the declarations another file exports. At the top of a
file:

* `import * as geom from "./geom";` reads `geom.isc`; what it exports is
  then reached as `geom.f`, `geom.T` and `geom.C`, in expressions, types and
  patterns;
* `import { area, Shape, Circle as Disc } from "./geom";` makes these names
  usable without a prefix (`as` renames one).

A path that starts with `./` or `../` is relative to the importing file
(`"./lib/util"` reads `lib/util.isc`); a bare name (`"util"`) is looked for
in the directories listed in `$ISC_PATH` (separated by `:`). Only what a
module declares with `export` can be reached from another file: a
function, a type (and its constructors), a type alias, a constant or a
`declare function`.

```isc-file geom.isc
// geom.isc
export type Shape = Circle(f64) | Square(f64);

function sq(x: f64): f64 {
    return x * x;
}

export function area(s: Shape): f64 {
    return match (s) {
        Circle(r) => 3.0 * sq(r),
        Square(c) => sq(c),
    };
}
```

```isc
// main.isc
import * as geom from "./geom";
import { area, Square } from "./geom";

action main(args: string[k]): i32 {
    const c: geom.Shape = geom.Circle(2.0);
    print(geom.area(c));
    print(area(Square(3.0)));
    match (c) {
        Square(s) => print(s),
        _ => print("not a square"),
    };
    return 0;
}
```

```output
12
9
not a square
```

Inside a module, an unqualified name is looked for among the module's own
declarations first, then among the names it imports by name: `geom.isc`
calls `sq` and could declare its own `sum` without changing the `sum` of
the program. A name imported by name comes after the local variables: a
`const area = ...` hides the imported `area` in its scope, as in
TypeScript. A module is one file: read once whatever the number of files
and the paths that import it, so its types are the same everywhere. An
import cycle, a missing module, a name the module does not export, a name
both imported and declared, and a module that declares `main` are compile
errors. In the importing file, `geom.x` always means the module, even
where a variable is named `geom`. A `declare function` names a C function:
it keeps its C name and is global to the program.

### Functions chosen at run time

A function value may depend on a condition known only when the program
runs: `const f = c ? g : h;`. No closure is built: the condition is
computed once, where `f` is bound, and each call of `f` becomes a branch
between `g` and `h`. A `match` can choose a function too, one per arm
(the first arm that matches); the function may read the variables of its
pattern, at any depth (`Scale(c) => (v) => v * c`, `Cons(a, Cons(b, _)) =>
(v) => v * a + b`). The memory report lists each such choice.

```isc
function double(v: f64): f64 {
    return v * 2.0;
}

function square(v: f64): f64 {
    return v * v;
}

action main(args: string[k]): i32 {
    const x = map(iota(4), (i) => f64(i));
    const f = sum(x) > 5.0 ? double : square;
    print(map(x, f));
    print(map(x, sum(x) > 100.0 ? double : (v) => v + 1.0));
    return 0;
}
```

```output
[0, 2, 4, 6]
[1, 2, 3, 4]
```

### Functions in structures

A tuple, a record or an array can hold functions. They exist only while the
program is compiled: a field or a pattern gives the function itself, and
the call is specialized as for an argument. An array of functions indexed
by a value known at run time, `fs[i](x)`, is a branch on `i`; an index out
of bounds is the usual panic. The memory report lists each such branch.

```isc
function double(v: f64): f64 {
    return v * 2.0;
}

function square(v: f64): f64 {
    return v * v;
}

function applyBoth(ops: [(f64) => f64, (f64) => f64], x: f64): f64 {
    const [f, g] = ops;
    return f(x) + g(x);
}

action main(args: string[k]): i32 {
    print(applyBoth([double, square] as const, 3.0));
    const r = { up: double, sq: square };
    print(r.sq(5.0));
    const fs = [double, square, (v) => v + 1.0];
    const i = i64(sum([1.0, 1.0]));
    print(fs[i](10.0));
    print(map([1.0, 2.0], fs[i - 1]));
    return 0;
}
```

```output
15
25
11
[1, 4]
```

A value of a sum type can hold a function in a field of its own
(`Apply((x: f64) => f64)`, `Option<(x: f64) => f64>`, a recursive type).
The value holds a small code: the index of its function among those the
program stores in that field, and a call is a branch on the code, as for
an array of functions. Such a function cannot read a variable or a size of
its context (a compile error says so: store that value in another field);
a value that holds a function is neither printed nor compared.

```isc
type Op = Apply((x: f64) => f64) | Const(f64);

type Pipe = End | Step((x: f64) => f64, Pipe);

function run(op: Op, x: f64): f64 {
    return match (op) {
        Apply(f) => f(x),
        Const(c) => c,
    };
}

function through(p: Pipe, x: f64): f64 {
    return match (p) {
        End => x,
        Step(f, rest) => through(rest, f(x)),
    };
}

action main(args: string[k]): i32 {
    const ops = [Apply((v) => v + 1.0), Const(7.0), Apply(Math.sqrt)];
    print(ops.map((o) => run(o, 16.0)));
    print(through(Step((v) => v * 2.0, Step(Math.sqrt, End)), 8.0));
    return 0;
}
```

```output
[17, 7, 4]
4
```

### Functions returned by functions

A function can return a function. The function that returns it is inlined
where it is called, like any function that is not recursive; the values
the returned function reads (the arguments, the `const`s) are computed
once, where it is made, not at each call:

```isc
function adder(k: f64): (x: f64) => f64 {
    return (x) => x + k;
}

function compose(f: (x: f64) => f64, g: (x: f64) => f64): (x: f64) => f64 {
    return (x) => g(f(x));
}

action main(args: string[k]): i32 {
    const add3 = adder(3.0);
    print(add3(1.5));
    print([1.0, 2.0, 3.0].map(compose(adder(1.0), (x) => x * 2.0)));
    return 0;
}
```

```output
4.5
[4, 6, 8]
```

A recursive function cannot return a function: it is not inlined.

### Recursion

A recursive function is not inlined. A call of itself in tail position (the
last thing it does) is a loop: it does not use more stack, however many
times it repeats, even when it passes a new array each time. Any other call
uses the stack; a recursion deeper than the stack can hold is a panic,
`stack overflow: too many nested calls of ...`. The memory report lists each
call and says which ones are loops. A recursive function may take a
function: it is compiled once for each function it is given (a function of
the program, an operator, a primitive or a lambda); the values and the
sizes a lambda reads from its context become parameters of that compiled
function.

```isc
function fib(n: i64): i64 {
    if (n < 2) {
        return n;
    } else {
        return fib(n - 1) + fib(n - 2);
    }
}

function even(n: i64): boolean {
    if (n == 0) {
        return true;
    } else {
        return odd(n - 1);
    }
}

function odd(n: i64): boolean {
    if (n == 0) {
        return false;
    } else {
        return even(n - 1);
    }
}

function halveSum(x: f64[n]): f64 {
    if (n <= 4) {
        return sum(x);
    } else {
        return halveSum(x.slice(0, n / 2)) + halveSum(x.slice(n / 2, n));
    }
}

function countDown(n: i64, acc: i64): i64 {
    if (n == 0) {
        return acc;
    } else {
        return countDown(n - 1, acc + n);
    }
}

action main(args: string[k]): i32 {
    print(fib(20));
    print(even(1001));
    print(halveSum(map(iota(100), (i) => f64(i))));
    print(countDown(1000000, 0));
    return 0;
}
```

```output
6765
false
4950
500000500000
```

A tail call to another function reuses the frame too, so that functions
that call each other in tail position (`even` and `odd` above) do not use
more stack either, as long as the arrays they pass are their own
parameters. `requireTail` accepts such calls.

## Types

| Type | Values |
| --- | --- |
| `i8` `i16` `i32` `i64` | signed integers, two's complement, arithmetic modulo 2^bits |
| `u8` `u16` `u32` `u64` | unsigned integers, arithmetic modulo 2^bits |
| `f64` | IEEE 754 binary64 |
| `f32` | IEEE 754 binary32 |
| `boolean` | `true`, `false` |
| `bigint` | integers of any size: literals `123n`, `0xffn` (see [Big integers](#big-integers)) |
| `void` | the result of an action that returns nothing |
| `string` | immutable UTF-8 text: literals `"..."`, the command-line arguments (see [Strings](#strings)) |
| `[A, B, ...]` | tuples: `[a, b] as const`, taken apart with `const [a, b] = ...` |
| `{a: A, b: B}` | records: fields by name; the order of the fields does not matter |
| `Option<T>`, ... | sum types declared with `type` (`Option` is in the prelude) |
| `Map<K, V>` | a dictionary (see [Maps](#maps)) |
| `T[n]` | arrays of `n` elements of a scalar type (integers, floats, `boolean`), `string`, or a tuple of these |
| `T[r][c]` | `r` rows of `c` elements (see [Windows and arrays of arrays](#windows-and-arrays-of-arrays)) |
| `(a: A) => B`, `(A, B) => C` | functions, as parameters of a `function` |

There is no implicit conversion between values. The conversion functions
`i8` ... `u64`, `f32` and `f64` take an integer or a floating-point value:

* integer to integer: modulo 2^bits (`u8(300)` is 44, `i8(200)` is -56);
* integer to `f32` or `f64`, and `f64` to `f32`: rounded to nearest;
* floating-point to integer: truncated toward zero, saturated to the range of
  the type, NaN gives 0 (`i32(1e10)` is 2147483647);
* `f32` to `f64`: exact.

### Literals

* Integers: `42`, `1_000_000`, `0xff`; floats: `2.5`, `1e9`, `2.5e-3` (a digit
  is required on both sides of the point: `1.` and `.5` are not literals).
* A suffix fixes the type: `42u8`, `7i32`, `1.0f64`.
* Without a suffix, a literal takes the type its context requires
  (`c[v] + 1` with `c[v]: u32` makes `1` a `u32`; `2 * x` with `x: f64` makes
  `2` a `f64`). If the context fixes nothing: `i64` for an integer, `f64` for a
  float. A literal out of the range of its type is a compile error.

```isc
action main(args: string[k]): i32 {
    const a: u8 = 250;
    print(a + 10); // modulo 2^8
    print(i64(a) + 10);
    print([i32(3.9), i32(-3.9)] as const); // truncated toward zero
    print(u8(-1));
    print(7 / 2);
    print(-7 % 2); // the sign of the dividend
    print(7.0 / 2.0);
    return 0;
}
```

```output
4
260
(3, -3)
255
3
-1
3.5
```

`f32` values are computed in binary32: every operation is rounded to
binary32, as IEEE 754 requires, and a literal of type `f32` is rounded
directly from its decimal text (never through `f64`).

### Big integers

A `bigint` is an integer of any size, as in TypeScript: literals `123n`,
`-0xffn`, the operators `+ - * / % **` (`/` truncates toward zero, `%` has
the sign of the dividend, a division by `0n` or a negative exponent is a
panic), the comparisons, `BigInt(x)` of an integer or of a text, `i64(b)`
(modulo 2^64), `f64(b)` and `Number(b)` (the nearest float), `String(b)`
and `${b}`. A `bigint` does not mix with the other numbers: `1n + 1.5` is a
type error. Like a string, it is owned and freed; an array of them is
sorted by `sort`, and the generic functions of numbers (`sum`) take it.

```isc
function factorial(n: i64, acc: bigint): bigint {
    if (n <= 1) return acc;
    return factorial(n - 1, acc * BigInt(n));
}

action main(args: string[k]): i32 {
    print(factorial(25, 1n));
    print(2n ** 70n / 3n);
    print([10n, -2n, 7n].sum());
    return 0;
}
```

```output
15511210043330985984000000
393530540239137101141
15
```

### Strings

A `string` is immutable UTF-8 text. A literal is written `"..."` on one line,
with the escapes `\n`, `\t`, `\r`, `\0`, `\\`, `\"` and `\u{hex}` (a Unicode
scalar value). Strings are values like the others: in a `function`, in arrays,
tuples, records and sum types, compared with `==` and ordered by their bytes
(`<`, `<=`, `>`, `>=`, lexicographic, so `"Z" < "a"`).

| Function | Type | Result |
| --- | --- | --- |
| `a + b`, `concat(a, b)` | `(string, string) => string` | the concatenation |
| `s.length`, `len(s)` | `(string) => i64` | the number of bytes (not of characters); `x.length` of an array: its number of elements |
| `lines(s)` | `(string) => string[k]` | the lines (a `\r` before a `\n` removed; no empty last line) |
| `split(s, sep)` | `(string, string) => string[k]` | the parts between the separators (`sep` empty: a panic) |
| `trim(s)` | `(string) => string` | without the white space at both ends |
| `join(xs, sep)` | `(string[n], string) => string` | the strings with `sep` between them |
| `bytes(s)` | `(string) => u8[k]` | the bytes |
| `stringOf(b)` | `(u8[n]) => string` | the text of these bytes; a panic if they are not UTF-8 |
| `String(x)`, `x.toString()` | `(scalar) => string` | the text `print` writes (a string is itself) |
| `parseI64(s)`, `parseF64(s)` | `(string) => i64`, `(string) => f64` | a decimal integer (`+5`, `-12`), a decimal number (`2.5`, `-1e-3`, `inf`, `NaN`); anything else is a panic (`tryParseI64`, `tryParseF64`: a `Result`) |
| `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` | `(string) => f64` | as ECMAScript: `Number` of the whole text (`""` is 0, `"0x1F"` is 31), `parseFloat` and `parseInt` of its longest prefix; NaN when there is no number |
| `s.indexOf(t)` | `(string, string) => i64` | the byte index of the first `t`, -1 if none |
| `s.includes(t)`, `s.startsWith(t)`, `s.endsWith(t)` | `(string, string) => boolean` | |
| `s.replace(a, b)`, `s.replaceAll(a, b)` | `(string, string, string) => string` | the first `a`, every `a`, replaced by `b` (`a` empty: `b` around each character; `b` has the `$` patterns of [Regular expressions](#regular-expressions)) |
| `s.toUpperCase()`, `s.toLowerCase()` | `(string) => string` | the case mappings of Unicode, without a locale (`"straße".toUpperCase()` is `"STRASSE"`) |
| `s.padStart(n, fill)`, `s.padEnd(n, fill)` | `(string, i64, string) => string` | padded to `n` bytes with copies of `fill` (a space when omitted) |
| `s.repeat(n)` | `(string, i64) => string` | `n` copies (`n < 0`: a panic) |
| `s.trimStart()`, `s.trimEnd()` | `(string) => string` | without the white space at one end |
| `s.slice(a, b)` | `(string, i64, i64) => string` | the bytes `a` to `b - 1`; bounds outside or inside a character: a panic |
| `x.toFixed(d)` | `(f64, i64) => string` | `d` decimals (0 to 100), the exact value rounded to the nearest, a half away from zero |

Each of these functions can also be called as a method: `s.trim()`,
`words.join(", ")`. They mean what they mean in TypeScript (the ECMAScript
standard), with one difference: a length or an index counts the bytes of
UTF-8, not the units of UTF-16. `toFixed` from 1e21 on writes the number as
`print` does (TypeScript writes `1e+21`).

```isc
action main(args: string[k]): i32 {
    const s = "Hello, World";
    print([s.indexOf("World"), s.indexOf("x")] as const);
    print(s.replaceAll("l", "L").toUpperCase());
    print("7".padStart(3, "0") + "|" + "ab".padEnd(4) + "|");
    print(s.slice(7, 12));
    print([(1.005).toFixed(2), (0.125).toFixed(2), (-2.5).toFixed(0)]);
    return 0;
}
```

```output
(7, -1)
HELLO, WORLD
007|ab  |
World
[1.00, 0.13, -3]
```

```isc
action main(args: string[k]): i32 {
    const s = "  Isocron v1  ";
    const t = s.trim();
    print([t.includes("cron"), t.startsWith("Iso"), t.endsWith("v2")] as const);
    print(`[${s.trimStart()}] [${s.trimEnd()}] ${t.toLowerCase()}`);
    print([String(2.5), (42).toString(), String(true)].join("/"));
    print([Number("0x1F"), Number(""), parseFloat("3.5kg"), parseInt("ff", 16), parseInt("12.9")]);
    print(Number("abc"));
    return 0;
}
```

```output
(true, true, false)
[Isocron v1  ] [  Isocron v1] isocron v1
2.5/42/true
[31, 0, 3.5, 255, 12]
NaN
```

A template literal `` `n = ${n}` `` is the concatenation of its texts and of
its insertions, each written as `print` writes it: a scalar or a string (an
array is a compile error). It may span several lines; `` \` `` and `\${`
write the characters themselves.

```isc
function plural(n: i64, word: string): string {
    return `${n} ${n == 1 ? word : word + "s"}`;
}

action main(args: string[k]): i32 {
    const words = ["pomme", "kiwi", "abricot"];
    print(plural(3, "fruit"));
    print("héllo".length); // bytes, not characters
    print("abc" < "abd");
    print(words.filter((w) => w < "l"));
    print(words.fold("", (acc, w) => acc + w + ";"));
    print(bytes("AZ"));
    print(stringOf([79u8, 75u8]));
    print("quote: \"x\", ☺");
    return 0;
}
```

```output
3 fruits
6
true
[kiwi, abricot]
pomme;kiwi;abricot;
[65, 90]
OK
quote: "x", ☺
```

Each operation makes a new string, owned and freed as an array is; a
string read from an array or a parameter is borrowed, and copied only when
it must outlive its owner (see [Memory](#memory)). `reduce` and `scan` of
strings follow the fixed order of section 10, so with a non-commutative
operator such as `+` of strings the result of `reduce` is not the
left-to-right concatenation: use `fold` for that.

```isc-panic
action main(args: string[k]): i32 {
    print(stringOf([104u8, 255u8]));
    return 0;
}
```

```output
panic at line 2, column 19: stringOf: the bytes are not UTF-8 (invalid from byte 1)
```

```isc
action main(args: string[k]): i32 {
    const x: f32 = 0.1;
    print(x + 0.2); // rounded to binary32
    print(f64(x)); // the binary32 value of 0.1, exactly
    print(sum(map(iota(10), (i) => f32(i) * 0.1f32)));
    print(sqrt(2.0f32));
    print(i32(f32(16777217))); // 2^24 + 1 is not a binary32
    return 0;
}
```

```output
0.3
0.10000000149011612
4.5
1.4142135
16777216
```

### Sizes

An array type carries its size: `f64[n]`, `u32[256]`, `i64[n - 1]`,
`f64[2 * n]`. In the current compiler, each size variable must appear alone as
the size of an array parameter (`x: T[n]`); the size of a result may be any
sum or product of these variables and of constants.

* Sizes the compiler can compare are checked at compile time: passing a
  `f64[3]` where a `f64[4]` is expected is an error.
* Sizes known only at run time (arrays read from files, for example) are
  checked when a function is called and by the element-wise operators; a
  mismatch is a panic.

A parameter declared `k: size` is an `i64` that is also a size of the
signature: the size of the result can depend on it. Its argument is a size
(a literal, a size variable, a sum or product of these) or a variable (bind
any other expression with `const` first); a negative value is a panic.

```isc
function firstK(x: f64[n], k: size): f64[k] {
    return map(iota(k), (i) => x[i]);
}

function repeat(v: f64, k: size): f64[k] {
    return fill(k, v);
}

action main(args: string[k]): i32 {
    print(firstK([1.0, 2.0, 3.0], 2));
    const m = 3;
    print(repeat(0.5, m));
    return 0;
}
```

```output
[1, 2]
[0.5, 0.5, 0.5]
```

```isc-panic
function repeat(v: f64, k: size): f64[k] {
    return fill(k, v);
}

action main(args: string[k]): i32 {
    const m = 1 - 3;
    print(repeat(0.5, m));
    return 0;
}
```

```output
panic at line 7, column 17: negative size: -2
```

```isc
function add(a: f64[n], b: f64[n]): f64[n] {
    return a + b;
}

action main(args: string[k]): i32 {
    print(add([1.0, 2.0], [10.0, 20.0]));
    return 0;
}
```

```output
[11, 22]
```

## Expressions

### Operators

From lowest to highest precedence, as in TypeScript (the ECMAScript standard):

| Operators | Associativity |
| --- | --- |
| `c ? a : b` | right |
| `a ?? b` (of an `Option`, see [Maps](#maps)) | right |
| `\|\|` | left |
| `&&` | left |
| `\|` (bitwise or) | left |
| `^` (bitwise exclusive or) | left |
| `&` (bitwise and) | left |
| `==` `!=` (also written `===` `!==`) | none |
| `<` `<=` `>` `>=` | none: `a < b < c` is an error |
| `<<` `>>` | left |
| `+` `-` | left |
| `*` `/` `%` | left |
| prefix `-`, `!` | — |
| `**` (power) | right |
| calls `f(x)`, indexing `x[i]`, `.` (field, method), `as const` | left |

* Integer `/` truncates toward zero and `%` has the sign of the dividend;
  dividing by zero is a panic. `MIN / -1` is `MIN` and `MIN % -1` is `0`.
* On `f64`, `%` is the remainder of C's `fmod`.
* `x ** k` takes an integer exponent `k >= 0` (a negative `k` is a panic) and
  is computed by binary exponentiation from the most significant bit:
  `x ** 2` is exactly `x * x`. With a floating-point exponent, `x ** y` is
  `pow(x, y)` (see [Performance model](#performance-model)).
  As in TypeScript, `-x ** 2` is an error: write `-(x ** 2)` or `(-x) ** 2`.
* `&&` and `||` evaluate their right operand only when needed.
* `==` and `!=` compare values, including arrays (arrays of different lengths
  are different) and tuples; `===` and `!==` are the same operators.
* `+` adds numbers and concatenates strings.
* `Math.floor`, `Math.ceil`, `Math.trunc`, `Math.round` and `Math.abs` (also
  without `Math.`) are those of TypeScript: `Math.round` rounds a half toward
  +inf (`Math.round(-2.5)` is -2, `Math.round(-0.25)` is -0); `abs` takes an
  integer too (modulo 2^bits: the minimum is itself).
* `&`, `|`, `^`, `<<` and `>>` take integers of the same type, as `+` does
  (element-wise on arrays too). They are below the comparisons, as in
  TypeScript: `x & 1 == 0` is `x & (1 == 0)`, a type error; write
  `(x & 1) == 0`. `>>` is arithmetic on signed types and
  logical on unsigned ones; the shift amount is taken modulo the number of
  bits (`1 << 65` is `2` on `i64`).

```isc
action main(args: string[k]): i32 {
    print([12 & 10, 12 | 10, 12 ^ 10] as const);
    print([1u8 << 7, -128i8 >> 7, 128u8 >> 7] as const);
    print(fold([true, false, true], 0u8, (acc, b) => acc << 1 | (b ? 1u8 : 0u8)));
    return 0;
}
```

```output
(8, 14, 6)
(128, -1, 1)
5
```

```isc
action main(args: string[k]): i32 {
    print([Math.floor(-2.5), Math.ceil(-2.5), Math.round(-2.5), Math.round(2.5), Math.trunc(-2.7)]);
    print([Math.abs(-3), Math.abs(-1.5) == 1.5 ? 1 : 0] as const);
    print([Math.tan(0.0), Math.asin(1.0) * 2.0, Math.acos(1.0), Math.atan(1.0) * 4.0, Math.atan2(1.0, -1.0)]);
    return 0;
}
```

```output
[-3, -2, -2, 3, -2]
(3, 1)
[0, 3.141592653589793, 0, 3.141592653589793, 2.356194490192345]
```

### `const`, `if`, lambdas, methods

```isc
function clamp01(x: f64): f64 {
    if (x < 0.0) return 0.0;
    if (x > 1.0) return 1.0;
    return x;
}

action main(args: string[k]): i32 {
    const [lo, hi] = [clamp01(-2.0), clamp01(7.0)] as const;
    const square = (v) => v * v; // a lambda bound to a name
    print([lo, hi] as const);
    print(square(1.5));
    print(sum(map(iota(5), (i) => i * i)));
    return 0;
}
```

```output
(0, 1)
2.25
30
```

* In a `function`, an `if` without `else` must `return`: the statements
  after it are its other branch. `if (c) { ... } else { ... }` and
  `c ? a : b` are the general forms.
* `const [a, b] = t;` takes a tuple apart; `const { x, y } = r;` a record.
* A lambda is written `(a, b) => body` or `a => body`, where `body` is an
  expression or a block `{ ... return ...; }`; its parameters may be annotated
  (`(v: f64) => v * 2.0`), otherwise their types are inferred. It can capture
  variables of its context.
* `x.f(a, b)` is `f(x, a, b)` when `x` is neither a record nor a module:
  `xs.map((v) => v + 1.0)`, `x.sum()`, `words.join(", ")`. `x.length` is
  `len(x)`.
* Names follow TypeScript's conventions: camelCase for the functions of
  the standard library (`readF64`, `parseCsvF64`, `printBytes`), and
  TypeScript's own names where the meaning is the same (`String(x)`,
  `x.toString()`, `Math.sqrt`, `Math.min`, `Math.PI`). `Math.f` is always
  the mathematical function, even where a variable is named `Math`; the
  same functions can be called without `Math.` (`sqrt(x)`).
* The predefined operators can be passed as functions: `(+)`, `(-)`, `(*)`,
  `(/)`, `(%)`, `(==)`, `(!=)`, `(<)`, `(<=)`, `(>)`, `(>=)`, `(&&)`, `(||)`.

## Arrays

### Building arrays

| Expression | Result |
| --- | --- |
| `[e1, e2, ...]` | an array literal |
| `iota(n)` | `[0, 1, ..., n - 1]` as `i64[n]` |
| `fill(n, v)` | `n` copies of `v` |
| `zeros()` | an array of zeros whose size and element type come from the context |
| `map(x, f)`, `x.map(f)` | `[f(x[0]), ..., f(x[n - 1])]` |
| `zip(a, b)` | the pairs `[a[i], b[i]]` of two arrays of the same size, as tuples |
| `concat(a, b)` | the elements of `a`, then those of `b`: a `T[n]` and a `T[m]` give a `T[n + m]` (`+` of two arrays adds them element by element) |

The length given to `iota` and `fill` is a size (an integer literal, a size
variable, a sum or product of these: the type of the result knows it) or any
`i64` (`iota(n)` of a `const n`, `fill(args.length, "")`): the size is then
known at run time, and a negative length is a panic.

### Regular expressions

A `RegExp` is written `/source/flags` (where an expression starts) or
`RegExp(source, flags)`; the flags are `g`, `i`, `m`, `s` and `u`.
`re.test(s)`, `s.search(re)` (a byte index, -1 if none), `s.replace(re,
to)` (every match with `g`, else the first), `s.replaceAll(re, to)` (`g`
required, as in TypeScript: checked at compilation when the flags are
written), `countMatches(s, re)` and `matches(s, re)`
(the texts of the matches, left to right, without overlap). In `to`, `$&`
is the match, `$1` to `$99` its groups, `` $` `` and `$'` the text before
and after, `$$` a `$`; with a text as the pattern, `s.replace(a, b)`
replaces the first `a` and the patterns of `b` hold too. The engine runs in
linear time (that of the crate `regex`): a backreference or a look-around
is refused, at compilation for a literal. `\d`, `\w` and `\s` are those of
Unicode. A `RegExp` is neither printed nor compared.

```isc
action main(args: string[k]): i32 {
    const dna = "GGCCaggGTAAAtttACCCTtaggGTAAA";
    print(countMatches(dna, /agggtaaa|tttaccct/gi));
    print("2024-11-03".replace(/(\d+)-(\d+)-(\d+)/, "$3/$2/$1"));
    print(matches("a1b22c333", /\d+/g));
    print(["abc", "a1"].map((w) => /^[a-z]+$/.test(w)));
    return 0;
}
```

```output
3
03/11/2024
[1, 22, 333]
[true, false]
```

`RegExp(source, flags)` and `new RegExp(source, flags)` build one from a
text known at run time; an invalid one is then a panic:

```isc
action main(args: string[k]): i32 {
    const word = "c.t";
    const re = new RegExp(word.replaceAll(".", "[a-z]"), "g");
    print(matches("cat cut cart cot", re));
    print("one two three".search(/t[wh]/));
    print(RegExp("^\\d+$").test("2026"));
    return 0;
}
```

```output
[cat, cut, cot]
4
true
```

### Sorting and random numbers

`sort(x)` and `sortBy(x, key)` return a new array, sorted in a stable way
(equal elements keep their order) by the order of the type: integers and
booleans by value, floats with -0 before +0 and every NaN last, strings by
their bytes. Unlike TypeScript's `sort`, nothing is sorted in place and the
default order is not that of the texts of the numbers. `x.toReversed()` is
the reversed array.

`random(seed, i)` is the number `i` of the sequence of `seed`, uniform in
[0, 1): a function of its arguments (SplitMix64 on a counter), so that a
`map` over `iota(n)` draws `n` numbers in parallel, with the same bits on
every machine. `randomInt(seed, i, n)` is an integer in [0, n).

```isc
action main(args: string[k]): i32 {
    print(sort([5, -1, 3, 3]));
    print(["ccc", "a", "bb"].sortBy((w) => w.length));
    const xs = iota(4).map((i) => randomInt(42, i, 100));
    print(xs);
    print(sort(xs).toReversed());
    return 0;
}
```

```output
[-1, 3, 3, 5]
[a, bb, ccc]
[59, 16, 16, 4]
[59, 16, 16, 4]
```

```isc
action main(args: string[k]): i32 {
    print(iota(3).map((i) => random(7, i).toFixed(6)));
    print(random(7, 1) == random(7, 1));
    return 0;
}
```

```output
[0.524346, 0.302139, 0.940996]
true
```

### Maps

A `Map<K, V>` is a dictionary, never changed in place. `Map(entries)` (or
`new Map(entries)`, as in TypeScript) makes one from an array of pairs: each
key keeps the place of its first entry and takes the value of its last one,
the rule of `set` applied entry after entry. `countBy(xs, f)` is the map of
each key `f(x)` to its number of elements. Then:

* `m.size`, `m.has(k)`, `m.get(k)` (an `Option<V>`: `Some(v)` or `None`);
* `m.keys()`, `m.values()`, `m.entries()`: arrays, in the order of the
  first insertions, as the iterators of TypeScript;
* `m.set(k, v)` and `m.delete(k)`: a new map (in TypeScript, the same map
  changed).

`a ?? b` is the value of `a` if it is `Some`, else `b`, as `??` of
TypeScript for `undefined`; `a ?? b ?? c` tries `a`, then `b`. The keys
have the class `Key`: integers, booleans, strings and `bigint`; a float is
refused, its equality not being one of keys (NaN). A map is stored as three
arrays (keys, values, the keys in order): built at once from n entries with
m distinct keys in O(n + m log m) (a table of hashes, then the m keys
sorted), `get` and `has` in O(log m), `set` and `delete` rebuild it, so a
map is built from all its entries rather than by `set` in a loop. A map is neither
printed (print `m.entries()`), nor compared, nor stored in an array.

```isc
action main(args: string[k]): i32 {
    const words = "the cat and the dog and the bird".split(" ");
    const counts = countBy(words, (w) => w);
    print(counts.entries());
    print(counts.get("the") ?? 0);
    print(counts.get("fish") ?? 0);
    const ages = new Map([["ann", 31] as const, ["bob", 27] as const]);
    print(ages.set("cy", 40).delete("ann").entries());
    print(ages.has("ann"));
    return 0;
}
```

```output
[(the, 3), (cat, 1), (and, 2), (dog, 1), (bird, 1)]
3
0
[(bob, 27), (cy, 40)]
true
```

A function over maps declares its keys `Key`:

```isc
function invert<K extends Key, V extends Key>(m: Map<K, V>): Map<V, K> {
    return Map(m.entries().map((e) => {
        const [k, v] = e;
        return [v, k] as const;
    }));
}

action main(args: string[k]): i32 {
    const codes = new Map([["fr", 33] as const, ["de", 49] as const, ["it", 39] as const]);
    print(codes.keys());
    print(codes.values());
    const names = invert(codes);
    print(names.size);
    print(names.get(49) ?? "?");
    print(names.get(1) ?? "?");
    return 0;
}
```

```output
[fr, de, it]
[33, 49, 39]
3
de
?
```

### Element-wise operators

`+ - * / %`, unary `-`, `**` and the bitwise operators work element by
element on arrays of the same size, and an operand that is a scalar is
broadcast to every element: `x - m` subtracts `m` from every element of `x`.
They are equivalent to a `map` and fused with it: `sum((x - m) ** 2)` reads
`x` once and builds no intermediate array.

### Records

A record groups named fields. Two record types with the same fields are the
same type, whatever the order in which the fields are written. `r.f` reads a
field, `{ ...r, f: v }` is a new record where `f` is `v`, and a pattern
`{ f: p }` takes some fields apart (`{ f }` binds the field `f` to `f`). An array of records is stored in columns,
one per field, like an array of tuples.

```isc
function value(t: { price: f64, volume: u32 }): f64 {
    return t.price * f64(t.volume);
}

action main(args: string[k]): i32 {
    const t = { volume: 3u32, price: 2.5 };
    print(t);
    print({ ...t, price: 2.0 });
    const { price: p } = t;
    print(p);
    const trades = map(iota(3), (i) => ({ price: 10.0 + f64(i), volume: u32(i + 1) }));
    print(trades[2].volume);
    print(sum(map(trades, value)));
    print(t == { price: 2.5, volume: 3u32 });
    return 0;
}
```

```output
{price: 2.5, volume: 3}
{price: 2, volume: 3}
2.5
3
68
true
```

### Sum types and `match`

A `type` declaration names a sum type and its constructors; a constructor
may carry values, and the type may have parameters:

```isc
type Option<T> = None | Some(T);

type Shape = Circle(f64) | Rect(f64, f64) | Empty;

function area(s: Shape): f64 {
    return match (s) {
        Circle(r) => 3.0 * r * r,
        Rect(w, h) => w * h,
        Empty => 0.0,
    };
}

function safeDiv(a: f64, b: f64): Option<f64> {
    if (b == 0.0) {
        return None;
    } else {
        return Some(a / b);
    }
}

function classify(x: [i64, Option<boolean>]): i64 {
    return match (x) {
        [0, _] => 100,
        [n, Some(true)] => n,
        [n, Some(false)] => -n,
        _ => 7,
    };
}

action main(args: string[k]): i32 {
    const shapes = [Circle(1.0), Rect(2.0, 3.0), Empty];
    print(shapes);
    print(sum(map(shapes, area)));
    print([safeDiv(1.0, 4.0), safeDiv(1.0, 0.0)] as const);
    print([classify([0, None] as const), classify([5, Some(true)] as const), classify([5, None] as const)] as const);
    return 0;
}
```

```output
[Circle(1), Rect(2, 3), Empty]
9
(Some(0.25), None)
(100, 5, 7)
```

* `match` tries the arms in order and takes the first whose pattern matches.
  Patterns: `_`, a name (binds the value), a literal, a tuple `[p, q]`, a
  record `{ f: p }`, a constructor `C(p, ...)`. An arm is `pattern => value`;
  the value may be a block `{ ... return ...; }`.
* The arms must cover every value: an arm that always matches, or one arm
  per constructor (whose payload patterns always match), or `true` and
  `false`; otherwise the program does not compile.
* A `const` pattern must always match (`const [a, b] = ...`); a constructor
  pattern is allowed there only for a type with a single constructor.
* A value of a sum type is stored flat, without allocation: a tag and the
  values of every constructor (an array of them is stored in columns). A
  recursive type is stored differently: see the next section.

### Recursive types

A sum type may contain itself, directly or through other types. Its values
are nodes on the heap, each with a count of its owners: a value kept in two
places is not copied, its count goes up, and the node is freed when the
last owner lets it go. Freeing a long list takes no stack. Two values of a
recursive type are compared with `==` node by node (the floats as `==`
compares them: NaN differs from itself), without stack for a long list.
An array of a recursive type holds its nodes, each element one owner: an
element read and kept elsewhere, or an array copied, adds an owner.

```isc
type List = Nil | Cons(i64, List);

type Tree = Leaf(f64) | Node(Tree, Tree);

function build(n: i64, acc: List): List {
    if (n == 0) {
        return acc;
    } else {
        return build(n - 1, Cons(n, acc));
    }
}

function total(l: List, acc: i64): i64 {
    return match (l) {
        Nil => acc,
        Cons(h, t) => total(t, acc + h),
    };
}

function tree(d: i64): Tree {
    if (d == 0) {
        return Leaf(0.5);
    } else {
        return Node(tree(d - 1), tree(d - 1));
    }
}

function leaves(t: Tree): f64 {
    return match (t) {
        Leaf(v) => v,
        Node(a, b) => leaves(a) + leaves(b),
    };
}

action main(args: string[k]): i32 {
    const l = build(4, Nil);
    print(l);
    print(total(l, 0));
    print(leaves(tree(10)));
    return 0;
}
```

```output
Cons(1, Cons(2, Cons(3, Cons(4, Nil))))
10
512
```

### Arrays of tuples

An array of tuples is stored in columns, one column per component (in one
block of memory), so that a loop over one component reads contiguous memory.
`map`, `reduce`, `fold`, `fill`, indexing, updates and equality work on them
as on any array; tuples are taken apart with `const [a, b] = ...`:

```isc
function closest(a: [f64, i64], b: [f64, i64]): [f64, i64] {
    const [da, ia] = a;
    const [db, ib] = b;
    if (db < da) {
        return b;
    } else {
        return a;
    }
}

action main(args: string[k]): i32 {
    const pts = map(iota(6), (i) => [f64(i) - 3.2, i] as const);
    print(pts[1]);
    print(reduce(map(pts, (p) => {
        const [x, i] = p;
        return [x * x, i] as const;
    }), closest, [1e308, -1] as const));
    print(pts.with(0, [0.5, 99] as const));
    return 0;
}
```

```output
(-2.2, 1)
(0.04000000000000007, 3)
[(0.5, 99), (-2.2, 1), (-1.2000000000000002, 2), (-0.20000000000000018, 3), (0.7999999999999998, 4), (1.7999999999999998, 5)]
```

`zip(a, b)` pairs two arrays element by element. It builds no array, even
when it is used several times or inside a lambda: each `map`, `reduce`,
`fold` or indexing that reads it reads `a` and `b` directly. Only an action
that receives it, like `print`, builds the array of pairs.

```isc
function weighted(p: [f64, f64]): f64 {
    const [v, w] = p;
    return v * w;
}

action main(args: string[k]): i32 {
    const v = [2.0, 4.0, 6.0];
    const w = [0.5, 0.25, 1.0];
    print(zip(v, w));
    print(sum(map(zip(v, w), weighted)));
    return 0;
}
```

```output
[(2, 0.5), (4, 0.25), (6, 1)]
8
```

The two arrays must have the same size: a size the compiler cannot prove
equal is tested at run time, and a difference is a panic.

```isc-panic
action main(args: string[k]): i32 {
    const tags = map(args, (s) => 1);
    print(zip(tags, iota(2)));
    return 0;
}
```

```output
panic at line 3, column 14: size mismatch: 0 != 2
```

### Slices

`x.slice(a, b)` is the array of the elements of `x` from index `a` to index `b - 1`,
of size `b - a`; `a` and `b` are integers, and a slice with `a < 0`, `a > b`
or `b > n` is a panic. Its size is known at run time, like the size of an
array read from a file. A slice is a view: it reads the memory of `x`
and copies nothing, even when it is used several times or sliced again. It
is copied only when a value must own it: the target of `with`, a component
of a tuple, the result of a block that outlives `x`. A slice of an array of
tuples is still a copy. A slice of an array being computed, like
`x.map(f).slice(a, b)`, computes only the elements `a` to `b - 1`. Unlike
TypeScript's `slice`, the bounds are not clamped and a negative bound does
not count from the end: both are panics.

```isc
action main(args: string[k]): i32 {
    const x = [10.0, 20.0, 30.0, 40.0, 50.0];
    print(x.slice(1, 4));
    print(sum(x.slice(2, 5)));
    print(x.slice(3, 3));
    return 0;
}
```

```output
[20, 30, 40]
120
[]
```

### Windows and arrays of arrays

An array of arrays `T[n][m]` holds `n` rows of `m` scalars, in one block of
memory. `window(x, k)`, of type `T[n - k + 1][k]`, is the sliding windows of
`k` elements of `x`: row `i` is `x.slice(i, i + k)`. It is a view of `x`, never a
copy, and `k > n` is a panic. A row is read with `w[i]`, and `map`, `reduce`
and `fold` take the rows one by one.

```isc
function movingAverage(x: f64[n], k: size): f64[n - k + 1] {
    return map(window(x, k), (w) => sum(w) / f64(k));
}

action main(args: string[k]): i32 {
    const x = [1.0, 2.0, 3.0, 4.0, 5.0];
    print(window(x, 3));
    print(movingAverage(x, 2));
    print([[1, 2], [3, 4]][1]);
    return 0;
}
```

```output
[[1, 2, 3], [2, 3, 4], [3, 4, 5]]
[1.5, 2.5, 3.5, 4.5]
[3, 4]
```

A `map` whose function returns an array builds an array of arrays: its
rows have the same size (the type says so), each written in its place in
one block, without a temporary array for a row that is a literal, a `map`
or `iota`; a row that is a slice or another array is copied.

```isc
function outer(a: f64[n], b: f64[m]): f64[n][m] {
    return map(a, (u) => map(b, (v) => u * v));
}

action main(args: string[k]): i32 {
    const t = outer([1.0, 2.0], [1.0, 10.0, 100.0]);
    print(t);
    print(map(t, (r) => sum(r)));
    print(map([3, 4], (v) => [v, v * v]));
    return 0;
}
```

```output
[[1, 10, 100], [2, 20, 200]]
[111, 222]
[[3, 9], [4, 16]]
```

The elements of an array of arrays are scalars or strings, and its rows have
the same length: a `map` whose function returns rows of different lengths
(`lines(t).map((l) => l.split(","))` on lines with different numbers of
fields) is a panic, `size mismatch`.

### Reading and updating

* `x[i]` reads element `i`; any integer type can be an index; an index out of
  bounds is a panic.
* `x.with(i, v)` is a new array equal to `x` except at `i` (as TypeScript's
  `Array.prototype.with`). If `x` is not used anywhere else, the compiler
  updates it in place; otherwise it copies it. Write
  `requireUnique(x.with(i, v))` to make the in-place update a requirement:
  the program does not compile if `x` is used elsewhere. "Elsewhere" means
  on the same path: in `c ? x.with(i, v) : x`, each branch is the last use
  of `x` on its path, so the update is in place.

```isc
function setFirst(x: i64[n], v: i64): i64[n] {
    return x.with(0, v);
}

action main(args: string[k]): i32 {
    const a = iota(4);
    const b = setFirst(a, 100); // `a` is used again below: copied
    print(a);
    print(b);
    print(fill(3, true));
    print(map(a, (i) => f64(i) * 0.5));
    return 0;
}
```

```output
[0, 1, 2, 3]
[100, 1, 2, 3]
[true, true, true]
[0, 0.5, 1, 1.5]
```

## Reductions and their fixed order

### `reduce`

`reduce(x, op, e)` combines the elements of `x` with `op`, in an order fixed
by the language rather than left to right:

```
reduce_block(x[0..m]):                  // m <= B
    acc[j] = e                 for j = 0 .. V-1
    for i = 0 .. m-1:
        acc[i mod V] = op(acc[i mod V], x[i])
    for d = V/2, V/4, ..., 1:
        acc[j] = op(acc[j], acc[j + d])    for j = 0 .. d-1
    return acc[0]

reduce(x[0..n]):
    if n <= B: return reduce_block(x)
    else: reduce the results of reduce_block on consecutive blocks of B elements
```

with **V = 32** accumulators and **B = 4096** elements per block. This order
is what makes a floating-point sum both fast and reproducible:

* the 32 interleaved accumulators map to SIMD registers (four AVX-512 or eight
  AVX2 registers of doubles), so the compiler vectorizes the loop without
  reassociating anything;
* the blocks are independent, so they are computed on several cores, and the
  result does not depend on the number of cores;
* the result is the same on every target: a C compiler cannot vectorize a
  plain floating-point sum without changing its result (`-ffast-math`), while
  Isocron's result is the same at every optimization level.

Because `e` starts every accumulator, it should be the neutral element of
`op`. If it is not, the result is still deterministic, but it differs from a
left-to-right fold:

```isc
action main(args: string[k]): i32 {
    print(reduce(iota(3), (+), 0));
    print(reduce(iota(3), (+), 1)); // 1 is added in each of the 32 accumulators
    print(reduce(iota(40), (a, b) => a - b, 0)); // `-` is not associative
    print(fold(iota(40), 0, (a, b) => a - b)); // left to right
    return 0;
}
```

```output
3
35
0
-780
```

The standard library defines `sum(x) = reduce(x, (+), 0)`,
`product(x) = reduce(x, (*), 1)`, `mean(x) = sum(x) / f64(n)` (for `f64[n]`)
and `dot(a, b) = sum(a * b)`.

Several `reduce`s of the same array that do not depend on each other read it
once: in `max(x) - min(x)`, or in a function that computes `sum(x)` and
`sum(map(x, (v) => v * v))`, each block of `x` is reduced for both while it
is in the cache. Each result is the one its `reduce` gives alone. A `reduce`
that uses the result of another (`sum(map(x, (v) => v - m))` with `m =
mean(x)`) needs a second pass, and two statements of an action stay two
passes, since an action runs its statements in order.

### `filter`

`filter(x, p)` keeps the elements of `x` for which `p` is true, in their
order. The size of the result is known at run time (its type is
`T[k <= n]`).

```isc
action main(args: string[k]): i32 {
    const x = [3.0, -1.0, 4.0, -1.5, 5.0];
    print(filter(x, (v) => v > 0.0));
    print(sum(filter(x, (v) => v < 0.0)));
    print(filter(iota(10), (i) => i % 4 == 0));
    return 0;
}
```

```output
[3, 4, 5]
-2.5
[0, 4, 8]
```

A `filter` read by a `reduce`, a `fold` or a `map` builds no array: the
consumer reads the elements of `x` once, in order, and skips those that `p`
rejects. `sum(filter(x, p))` is the sum of the kept elements in the order
of section 10, exactly as if the filtered array had been built. An array is
built only when the elements are read by index (`x.filter(p)[2]`) or kept
as a value.

A function whose result has a size known only when it runs declares it with
a size variable that appears only in the result: `T[k]`, or `T[k <= n]` to
say that it does not exceed the size `n` of a parameter. Each call fixes it.

```isc
function positives(x: f64[n]): f64[k <= n] {
    return filter(x, (v) => v > 0.0);
}

function inner(x: f64[n]): f64[k] {
    return x.slice(1, n - 1);
}

action main(args: string[k]): i32 {
    const x = [3.0, -1.0, 4.0, -1.5, 5.0];
    print(positives(x));
    print(mean(positives(x)));
    print(inner(x));
    return 0;
}
```

```output
[3, 4, 5]
4
[-1, 4, -1.5]
```

### `scan`

`scan(x, op, e)` is the inclusive cumulation of `x`, from left to right:
`out[0] = op(e, x[0])`, `out[i] = op(out[i - 1], x[i])`, a new array of the
size of `x`. With `+`, `*`, `min` or `max` on integers, or `&&` or `||` on
booleans, it is computed in parallel (blocks reduced in parallel, then
cumulated again from the value before each block): these operators are
associative, so that the result is the one of the sequential cumulation for
any number of threads. Any other operator on integers or booleans is
cumulated sequentially.

A `scan` of floats has a fixed form, so that it runs in parallel with the
same result for any number of threads: its first block of 4096 elements is
cumulated from `e`, from left to right; every later block is cumulated from
its own first element, and each of its values is combined with the last
value of the block before: `out[i] = op(carry, local[i])`. Up to 4096
elements, it is the left-to-right cumulation.

```isc
action main(args: string[k]): i32 {
    print(scan(iota(6), (+), 0));
    print(scan([3, -1, 4, -1, 5], max, -100));
    print(scan([1.5, 2.0, 0.5], (a, b) => a * b, 1.0));
    print(scan([true, true, false, true], (p, q) => p && q, true));
    return 0;
}
```

```output
[0, 1, 3, 6, 10, 15]
[3, 3, 4, 4, 5]
[1.5, 3, 1.5]
[true, true, false, false]
```

### `scatter`

`scatter(dest, idx, vals, op)` is `dest` where each `vals[k]` is combined
into the element at `idx[k]`: `r[idx[k]] = op(r[idx[k]], vals[k])`, for k in
increasing order, so that several values at one index are combined in that
order. The indices are `i64`; one out of bounds is a panic.

```isc
action main(args: string[k]): i32 {
    print(scatter(fill(4, 0), [1, 3, 1, 0, 3, 3], [1, 1, 1, 1, 1, 1], (+)));
    print(scatter(fill(3, -1.0), [2, 0, 2], [5.0, 6.0, 7.0], (a, b) => a < 0.0 ? b : a));
    return 0;
}
```

```output
[1, 2, 0, 3]
[6, -1, 5]
```

### `count`

`count(x, p)` is the number of elements of `x` for which `p` is true, as
`i64`: `sum(map(x, (v) => p(v) ? 1 : 0))`, fused into one loop.

```isc
action main(args: string[k]): i32 {
    print(count(iota(100), (i) => i % 7 == 0));
    print(count([1.5, -2.0, 3.0], (v) => v > 0.0));
    return 0;
}
```

```output
15
2
```

### `min` and `max`

`min(a, b)` and `max(a, b)` take two numbers of the same type; `min(x)` and
`max(x)` take an array of numbers and are `reduce(x, min, e)` and
`reduce(x, max, e)`, in the fixed order of section 10, where `e` is `inf`
(`-inf` for `max`) for floats and the largest (smallest) value of an
integer type: it is the result for an empty array. On floats they are the
operations `minimum` and `maximum` of IEEE 754-2019: a NaN operand gives NaN,
and `-0` is below `0`, so that the result does not depend on the
instruction the processor has. `min` and `max` can be passed as functions.

```isc
action main(args: string[k]): i32 {
    const nan = 0.0 / 0.0;
    print([min(2.5, -1.0), max(2.5, -1.0), min(nan, 1.0)]);
    print(1.0 / min(0.0, -0.0));
    const x = [3.0, -4.5, 8.0, 0.5];
    print([min(x), max(x), reduce(x, max, 0.0)]);
    print(min(7, -2));
    print(max(u8(200), u8(100)));
    print(min(map(args, (s) => 1.0)));
    return 0;
}
```

```output
[-1, 2.5, NaN]
-inf
[-4.5, 8, 8]
-2
200
inf
```

### `fold`

`fold(x, init, f)` is the sequential left-to-right fold: `f(...f(f(init, x[0]),
x[1])..., x[n - 1])`. The accumulator can be of any type, including an array,
which is then updated in place:

```isc
function histogram(values: u8[n]): u32[4] {
    return fold(values, zeros(), (c, v) => requireUnique(c.with(v % 4, c[v % 4] + 1)));
}

action main(args: string[k]): i32 {
    print(histogram(map(iota(10), (i) => u8(i))));
    return 0;
}
```

```output
[3, 3, 2, 2]
```

**Rule P (privatized accumulation).** A `fold` whose function has the shape
`(c, v) => c.with(g(v), c[g(v)] + h(v))` (or `h(v) + c[g(v)]`), where the
elements of `c` are integers and neither `g` nor `h` uses `c`, is computed in
parallel: each thread accumulates a chunk of `x` into a private copy of zeros,
and the copies are added to `init`. Integer addition modulo 2^bits is
associative and commutative, so the result is the one of the sequential
`fold`, whatever the number of threads. The histogram above follows rule P.
Any other `fold` runs sequentially.

## Functions as arguments and generics

A `function` can take functions as parameters; every call is specialized at
compile time for the function it receives (no closure is allocated at run
time):

```isc
function twice(x: f64[n], f: (f64) => f64): f64[n] {
    return map(map(x, f), f);
}

function total<T extends Num>(x: T[n]): T {
    return reduce(x, (+), 0);
}

function largest<T extends Ord>(a: T, b: T): T {
    if (a > b) {
        return a;
    } else {
        return b;
    }
}

action main(args: string[k]): i32 {
    print(twice([1.0, 2.0], (v) => v + 0.5));
    print(twice([4.0, 16.0], sqrt));
    print(total([1u8, 2u8, 3u8]));
    print(total([0.5, 0.25]));
    print(largest(3, 7));
    return 0;
}
```

```output
[2, 3]
[1.4142135623730951, 2]
6
0.75
7
```

The classes of type parameters are `Num` (integers and `f64`: `+ - * / %`,
literals such as `0` and `1`, equality), `Ord` (ordered types: integers, `f64`,
`boolean`, `string`; `<`, `<=`, `>`, `>=`), `Eq` (equality) and `Key` (the
keys of a `Map`: integers, `boolean`, `string`, `bigint`, ordered, not
floats). A type
parameter takes its class with `extends`: `<T extends Num>`. A function can be passed
as an argument, chosen at run time and stored in a tuple, a record or an
array (see "Functions chosen at run time" and "Functions in structures"); a
function that returns a function is not compiled yet.

## Actions and input/output

Input/output is only possible in an `action`. The statements of an action run
in order, and the compiler never removes or reorders one. An action can call
other actions and functions; a `function` (and a lambda) cannot call an
action.

| Action | Type | Effect |
| --- | --- | --- |
| `readF64(path)` | `(string) => f64[k]` | reads a file of little-endian doubles (its size must be a multiple of 8) |
| `readBytes(path)` | `(string) => u8[k]` | reads a file as bytes |
| `writeF64(path, x)` | `(string, f64[n]) => void` | writes little-endian doubles; every NaN is written as the canonical NaN `0x7FF8000000000000` |
| `writeBytes(path, x)` | `(string, u8[n]) => void` | writes bytes |
| `readF32(path)`, `writeF32(path, x)` | as for `f64` | little-endian binary32 values (canonical NaN `0x7FC00000`) |
| `print(x)` | `(T) => void` | prints a value and a newline |
| `printBytes(x)` | `(u8[n]) => void` | writes the bytes to the standard output, as they are (binary output) |
| `readText(path)` | `(string) => string` | reads a UTF-8 text file (anything else is a panic) |
| `readStdin()`, `readStdinText()`, `readStdinLines()` | `() => u8[k]`, `() => string`, `() => string[k]` | the whole standard input: its bytes, its text (UTF-8, or a panic), its lines; read once (a later read finds its end) |
| `writeText(path, s)` | `(string, string) => void` | writes a text file |
| `readCsvF64(path, skip)` | `(string, i64) => f64[r][c]` | reads a CSV file of numbers, after `skip` header lines |
| `parseCsvF64(text, skip)` | `(string, i64) => f64[r][c]` | the same from a text |
| `clock()` | `() => u64` | a monotonic time in nanoseconds |
| `bench(r, f)` | `(i64, () => T) => [T, u64]` | calls `f` `r` times (`r >= 1`), returns the last result and the total time in nanoseconds |

The standard input is read once, whole: after `readStdinLines()`, a
`readStdinText()` finds its end.

```isc
action main(args: string[k]): i32 {
    const lines = readStdinLines();
    print(lines.length);
    print(lines.map((l) => l.split(",").map(parseI64).sum()));
    print(readStdinText().length);
    return 0;
}
```

```input
1,2,3
10,20
```

```output
2
[6, 30]
0
```

`printBytes` writes bytes as they are, without a newline; files are read
and written whole (this example needs a writable directory, so the guide
only type-checks it):

```isc
action main(args: string[k]): i32 {
    printBytes(bytes("no newline, "));
    printBytes([10u8]);
    return 0;
}
```

```output
no newline, 
```

```isc-check
action main(args: string[k]): i32 {
    const t0 = clock();
    writeText("notes.txt", "first\nsecond\n");
    const text = readText("notes.txt");
    print(lines(text).length);
    print(`${f64(clock() - t0) * 1e-9} s`);
    return 0;
}
```

A CSV file of numbers has one row per line and fields separated by commas;
spaces around a field are ignored, blank lines are skipped, and every row
must have as many fields as the first (a panic gives the line otherwise).
The header lines are read as text:

```isc
action main(args: string[k]): i32 {
    const text = "mass, size\n1.5, 2\n3, 4.25\n";
    const header = lines(text)[0].split(",").map(trim);
    const m = parseCsvF64(text, 1);
    print(header);
    print(m);
    print(iota(2).map((j) => m.map((r) => r[j]).sum() / f64(m.length)));
    return 0;
}
```

```output
[mass, size]
[[1.5, 2], [3, 4.25]]
[2.25, 3.125]
```

`print` writes integers in decimal and doubles in the shortest form that reads
back to the same value, never in exponent notation: `1` for `1.0`, `0.1`,
`-0`, `inf`, `NaN` (without sign: the bits of a NaN are never observable).

The programs of `examples/poc/` show the usual shape of a measured kernel:

```isc-check
// std_dev.isc    usage: std_dev <input.f64> <output.f64> <reps>
function stdDev(x: f64[n]): f64 {
    const m = mean(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

action main(args: string[k]): i32 {
    if (args.length < 3) {
        print("usage: std_dev <input.f64> <output.f64> <reps>");
        return 2;
    }
    const x = readF64(args[0]);
    const [r, ns] = bench(parseI64(args[2]), () => stdDev(x));
    print(ns);
    writeF64(args[1], [r]);
    return 0;
}
```

Without its arguments, the program prints its usage and exits with code 2;
reading `args[0]` when there is none would be a panic (an index out of
bounds).

### C functions: `declare function`

`declare function name(parameters): type;` declares a C function (System V x86-64
ABI), which only an action may call; the call is an effect, run in its
place, never removed. Its parameters are scalars (passed as the C
scalars of the same width; `boolean` as C's `bool`), `string` (passed as a
NUL-terminated copy, `const char *`) and arrays of scalars (passed as two
C parameters: a pointer to the elements, read only, and their number, an
`int64_t`); it returns a scalar or `void`. At most six parameters go in
integer registers (an array takes two) and eight in floating-point
registers; variadic C functions are not supported. A program is linked
with the C library and `libm`; `iscc build --link name` adds `-lname` (or
a library given by its path), and `iscc run --link name` loads the same
library into the interpreter.

```isc
declare function abs(v: i32): i32;

declare function toupper(c: i32): i32;

action main(args: string[k]): i32 {
    print(abs(-12i32));
    print(toupper(97i32));
    return 0;
}
```

```output
12
65
```

With `declare function strlen(s: string): u64;`, `strlen(args[0])` is the length of
the first argument of the program.

The C function `void scale(const double *x, int64_t n, double f)` would be
declared `declare function scale(x: f64[n], f: f64): void;`. A string that contains a
NUL byte cannot be passed: the call is a panic. The result of a C function
is that of the C library of the target: the interpreter and a native
program use the system's (glibc's `cbrt(27.0)` is `3.0000000000000004`), a
WebAssembly program WASI's (`3`).

## Memory

A program never allocates or frees memory: the compiler places each array,
frees it after its last use, and updates an array in place when nothing else
uses it. Every compilation checks these decisions: each new value is freed
exactly once, nothing is used after it is freed or given away, and no view
(a window, a row) outlives the array it reads. A check that fails is a
compilation error, which reports a defect of the compiler, not of the
program.

An array literal whose elements are all scalar constants (`[1, 2, 3]`,
`[0.5, 1.5]`, `const T = [true, false];`) is data of the program: reading
it allocates nothing, so `@requireNoAlloc` accepts it, and a value made of
it (`T.with(0, 9)`, the initial value of a `fold`) is a copy.

### The memory report

`iscc check file.isc --report` prints, and `iscc build file.isc --report`
prints and writes as JSON next to the executable (`file.report.json`), every
array the program allocates with the rule that causes it, every copy with
its reason, and the tests that remain at run time:

```isc-check
action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const x = map(a, (v) => v * 2.0);
    print(sum(x));
    print(max(x));
    const y = map(a, (v) => v + 1.0);
    const t = y.with(0, 5.0);
    print(sum(t) + sum(y));
    print(sum(map(a, (v) => v * v)) + max(a));
    return 0;
}
```

```text
Memory report — stats.isc
  line 2    alloc       f64[]         read from a file: an input of the program
  line 2    bounds      args[0]       checked at run time
  line 3    alloc       f64[]         shared result (rule M1): `x` is used 2 times, at lines 4, 5
  line 6    alloc       f64[]         shared result (rule M1): `y` is used 2 times, at lines 7, 8
  line 7    copy        f64[]         `y` is used again at line 8
  line 7    bounds      y.with(0, …)  checked at run time
```

An array is allocated only when a rule requires it: M1, a result used more
than once (`x`, read by `sum` and by `max`); M2, the result of `scan` or of
a `fold` that produces an array; M3, a value given to an action. A `const`
of an action is also computed before the next statement (section 12). The
`map` of line 10 has no entry: `sum` reads its elements as they are
computed, and `max(a)` is computed in the same pass over `a`. The update
of line 7 copies `y`, because `y` is read again at line 8; without that
line, `y` would be updated in place. A size known at compilation is
given in bytes. An entry inside a loop says so, since it happens at each
iteration. A line inside a function of the prelude (`sum`, `max`) is
reported at the line that calls it.

## Performance requirements

A requirement, written as a decorator before `function`, is a guarantee: if
the compiler cannot keep it, the program does not compile, and the error
says why.

* `requireTail`: every recursive call of the function is a tail call,
  compiled as a loop (a call whose result is used, a call through another
  function, or an array passed that is neither the same parameter nor a
  new one is refused).
* `requireFused`: no intermediate array is built inside the function; its
  result, written where its caller wants it, is not intermediate.
* `requireNoAlloc`: the function, and every function it calls, builds no
  array, copies none, and creates no node of a recursive type, beyond its
  result.

```isc
@requireTail
function gcd(a: u64, b: u64): u64 {
    if (b == 0) {
        return a;
    } else {
        return gcd(b, a % b);
    }
}

@requireNoAlloc
function norm2(x: f64[n]): f64 {
    return sqrt(sum(x * x));
}

@requireFused
function spread(x: f64[n]): f64 {
    const y = map(x, (v) => v * 2.0);
    return max(y) - min(y);
}

action main(args: string[k]): i32 {
    print(gcd(48, 18));
    print(norm2([3.0, 4.0]));
    print(spread([1.0, 5.0, 2.0]));
    return 0;
}
```

```output
6
5
8
```

```isc-error
@requireFused
function center(x: f64[n]): f64[n] {
    const y = x * 2.0;
    return y - mean(y);
}

action main(args: string[k]): i32 {
    print(center([1.0, 2.0]));
    return 0;
}
```

```output
requireFused not satisfied (line 2)
```

The explanation names each array: here `y` (line 3) is used twice (line 4),
so it is built once and kept (rule M1).

`@allowReassoc`, written at the same place, is a permission rather than a
requirement: inside the function (and the functions it calls), the
compiler may reassociate floating-point operations and contract `a * b +
c` into one fused multiply-add, as C's `-fassociative-math` does, not
`-ffast-math`: NaN and infinities keep their meaning, the sign of a zero
may not (as with `-fno-signed-zeros`, which `-fassociative-math`
requires). A `sum` or a `product` of a `filter` then counts each rejected
element as 0.0 (or 1.0), without moving the kept ones. The result may
differ in its last bits from one target to another; the memory report
lists each such function.

```isc
@allowReassoc
function fastDot(a: f64[n], b: f64[n]): f64 {
    return sum(a * b);
}

action main(args: string[k]): i32 {
    print(fastDot([1.0, 2.0], [3.0, 4.0]));
    return 0;
}
```

```output
11
```

## Errors and panics

Compile errors are printed as `file:line:column: error: message`, and `iscc`
exits with code 1:

```isc-error
function add(a: f64[n], b: f64[n]): f64[n] {
    return a + b;
}

action main(args: string[k]): i32 {
    print(add([1.0, 2.0, 3.0], [1.0, 2.0]));
    return 0;
}
```

```output
size mismatch
```

A *panic* stops the program with a message and the position of the failing
operation in the source, on the standard error, and the exit code 101. The
panics are: an index out of bounds, an integer division by zero, a negative
integer exponent, a size mismatch found at run time, `bench` with fewer than
one repetition, an invalid `parseI64` or `parseF64` argument, bytes that
are not UTF-8 given to `stringOf`, an input/output error, and running out of
memory.

```isc-panic
action main(args: string[k]): i32 {
    const x = iota(3);
    print(x[5]);
    return 0;
}
```

```output
panic at line 3, column 12: index out of bounds: the index is 5 but the length is 3
```

When several panics are possible, which one is reported is not specified; that
the program stops is guaranteed.

A panic that the literals of an operation make certain is a compile error,
with the same message, as in Rust: a division or a remainder of integers by
the literal `0`, a literal index outside an array literal, a negative
literal exponent of an integer, a negative literal size given to `iota` or
`fill`, a literal count of `repeat` below 0 or of `toFixed` outside 0 to
100, an invalid literal regular expression, `replaceAll` of a `RegExp`
whose written flags lack `g`. The check at run time remains for the
values known only then.

```isc-error
action main(args: string[k]): i32 {
    print([10, 20][2]);
    return 0;
}
```

```output
2:19: error: index out of bounds: the index is 2 but the length is 2
```

### Errors as values: `Result`

An action that can fail has a form that returns the error instead of
panicking. The prelude declares `type Result<T, E> = Ok(T) | Err(E)` and:

| Action | Result |
| --- | --- |
| `tryParseI64(s)`, `tryParseF64(s)` | `Result<i64, string>`, `Result<f64, string>` |
| `tryReadBytes(path)`, `tryReadF64(path)` | `Result<u8[k], string>`, `Result<f64[k], string>` |
| `tryReadText(path)` | `Result<string, string>` |
| `tryReadCsvF64(path, skip)` | `Result<f64[r][c], string>` |
| `tryWriteText(path, s)`, `tryWriteBytes(path, x)` | `Result<void, string>` |

The message of `Err` is the text the panic would print. What is read is
moved into `Ok`, never copied. A program that declares its own `Result`,
`Ok` or `Err` keeps it; the `try_` actions are then not available.

```isc
action main(args: string[k]): i32 {
    match (tryParseI64("12x")) {
        Ok(v) => print(v),
        Err(e) => print(concat("not a number: ", e)),
    };
    match (tryReadF64("/no/such/file")) {
        Ok(x) => print(sum(x)),
        Err(e) => print("no data"),
    };
    return 0;
}
```

```output
not a number: parseI64: `12x` is not a valid integer
no data
```

## Performance model

* **Fusion.** Element-wise operations, `map` and `iota` feeding a single
  consumer (`map`, `reduce`, `fold`, indexing) are fused into one loop: no
  intermediate array is built. Every function call is inlined, so fusion
  crosses function boundaries.
* **Vectorization.** `map`, element-wise operators and `reduce` (in its fixed
  order) are compiled to SIMD code for the host processor. With few vector
  registers (SSE: 16 registers of two doubles), the compiler updates the 32
  accumulators of a block in several passes; each accumulator still receives
  its elements in order, so the result does not change.
* **Loops inside an element.** In a `map`, a `fold` over the same elements
  for every element (`fold(iota(8), ...)`) and a call of a tail-recursive
  function are vectorized too: the function runs for 16 elements at once,
  each with its own state, and an element whose recursion has ended waits
  for the others (SPMD, DECISIONS D140). This holds when every recursive
  call is a tail call to the function itself, its parameters and result are
  scalars, and nothing in its body can panic whatever its arguments (no
  integer division by a variable, no indexing): `mandelbrot` in
  `examples/benchmarks_game/` runs about five times faster on one core than
  one element at a time.

```isc
function collatz(x: i64, steps: i64): i64 {
    if (x <= 1) {
        return steps;
    } else if (x % 2 == 0) {
        return collatz(x / 2, steps + 1);
    } else {
        return collatz(3 * x + 1, steps + 1);
    }
}

action main(args: string[k]): i32 {
    print(map(iota(10), (i) => collatz(i + 1, 0)));
    return 0;
}
```

```output
[0, 1, 7, 2, 5, 8, 16, 3, 19, 6]
```

* **Floating point.** IEEE 754, rounding to nearest: no implicit fused
  multiply-add (`a * b + c` rounds twice; `fma(a, b, c)` rounds once), no
  reassociation, no algebraic simplification, subnormals kept, `sqrt`
  correctly rounded. `exp`, `log`, `sin`, `cos`, `tan`, `asin`, `acos`,
  `atan`, `atan2` and `pow` (also written `x ** y` with a floating-point
  `y`), for `f64` and `f32`, come from the
  runtime of Isocron: the same code on every target and in the
  interpreter, so that their results are the same bits everywhere (they are
  within one unit in the last place, not always correctly rounded).

```isc
action main(args: string[k]): i32 {
    print(exp(1.0));
    print(fma(0.1, 10.0, -1.0));
    print(0.1 * 10.0 - 1.0);
    print(2.0 ** 0.5);
    print(map([0.0, 1.0], cos));
    return 0;
}
```

```output
2.7182818284590455
0.00000000000000005551115123125783
0
1.4142135623730951
[1, 0.5403023058681398]
```
* **Sliding windows.** In `map(window(x, k), (w) => ... reduce(w, op, z)
  ...)`, where `w` is used only by its `reduce` and `k` is a multiple of the
  vector width V (8 for `f64` with AVX-512) of at most 4096, neighbouring
  windows share the levels of their fixed-form trees: each level is
  computed once per position, about k / V + log2(V) operations per window
  instead of k, with the same bits as one `reduce` per window. `sum`,
  `max`, `min` and any operator of `reduce` take this path; other widths
  compute each window on its own.
* **Parallelism.** The blocks of `reduce`, the materialized element-wise loops
  and the folds of rule P run on a pool of threads; a `map` whose function
  is itself a loop (a `reduce` per row) is cut into one part per thread. A
  `reduce` of a `filter` runs on several threads too: each thread counts the
  kept elements of its part, then computes the blocks of kept elements
  that start in it. `ISC_THREADS=4 ./prog` sets the number of threads; by
  default, it is the number of CPUs the process may run on. The result is
  identical for any number of threads.
* **Large results.** A result larger than twice the L3 cache (32 MiB) is
  written without reading its lines first (non-temporal stores) by the
  integer `scan` and the windows of `map(window(...))`. A `scan` of integers
  with `+`, `*`, `min` or `max` computes 64 bytes at a time in a vector
  register.
* **Memory.** No garbage collector: arrays are freed at a point known at
  compile time; an array that is not used elsewhere is updated in place.
  On Linux the compiled programs use the allocator of Isocron: each thread
  keeps its own freed blocks, and a temporary array freed by one call of a
  kernel is given again, already mapped, to the next call; memory the
  program no longer uses goes back to the system. `ISC_ALLOC=glibc ./prog`
  uses glibc's allocator instead (WebAssembly always uses wasi-libc's).

## Not implemented yet

The specification describes more than the current compiler implements. Not
available yet (the compiler reports `... is not supported in the POC`):

* a recursive function that returns a function (D152);
* a function stored in a value of a sum type that reads a variable or a
  size of its context, and a lambda given to a recursive function that
  reads a function of its context (D153);
* arrays of arrays whose rows have different lengths, and arrays of arrays
  of arrays (D154).

`@requireVectorized` is outside the scope of the specification.

## The `iscc` command

```
iscc check <file.isc> [--report]            type-check (and print the memory report)
iscc ir    <file.isc>                       print the optimized IR
iscc run   <file.isc> [args...]             run with the reference interpreter
iscc build <file.isc> [-o out] [--emit-llvm f.ll] [--emit-asm f.s] [--debug]
           [--target native|wasm32-wasi|wasm32-wasi-threads|x86_64-windows]
iscc completions bash|zsh|fish              the completion of the shell
```

> [!TIP]
> `source <(iscc completions bash)` in `~/.bashrc` completes the commands,
> their options and values, and the `.isc` files. The messages are in
> colors, with the line of the source under an error, on a terminal
> (`--color auto|always|never` anywhere, `NO_COLOR` respected).

* `build` writes a native executable for the host processor (default name:
  the source file without `.isc`), stripped unless `--debug` is given. The
  linker is `$ISC_CC` (default `cc`).
* `build --target wasm32-wasi` writes a WebAssembly module for WASI preview 1
  (default name: `<file>.wasm`), with 128-bit SIMD. It runs with any WASI
  engine, for example `wasmtime run --dir=/::/ prog.wasm args...` (`--dir`
  gives the program access to the files it reads and writes; paths are then
  given as absolute paths). WASI preview 1 has no threads: the program runs
  on one thread.
* `build --target wasm32-wasi-threads` writes the same module with threads
  (wasi-threads: shared memory, atomics, `thread-spawn`). Only engines with
  these extensions run it, for example `wasmtime run -W threads=y -S
  threads=y --env ISC_THREADS=8 --dir=/::/ prog.wasm args...`; the number
  of threads is `ISC_THREADS`, 4 by default (WASI cannot tell the number of
  CPUs).
* `build --target x86_64-windows` writes a Windows executable (default
  name: `<file>.exe`) for the same instruction set as native code (the host
  CPU, or `--isa`): a console program with threads, reading and writing its
  files and standard streams as bytes (no `\r\n` added), its errors and
  panics with the messages of Linux. Its stack, and that of the threads, as
  on Linux. Linking uses `rust-lld` (the Rust target `x86_64-pc-windows-gnu`,
  `rustup target add x86_64-pc-windows-gnu`) and the start files and
  import libraries of MinGW-w64 (`$ISC_MINGW_DIR`, else `/usr`: Debian
  packages `mingw-w64-x86-64-dev` and `gcc-mingw-w64-x86-64-posix`); the
  program needs no DLL beyond those of Windows.
* Native (Linux and Windows), WebAssembly with or without threads, and the
  interpreter give the same results to the bit, and the same panic
  messages. Linking WebAssembly
  uses `rust-lld` and the WASI C library of the Rust targets `wasm32-wasip1`
  and `wasm32-wasip1-threads` (`rustup target add ...`).
* The interpreter is the reference: a compiled program prints the same output,
  writes the same files and panics with the same message.
* Experiment options, not part of the language: `--lanes V` (another number of
  accumulators for `reduce`; results depend on V, so such a program is not a
  conforming Isocron program; for `run`, it comes right after `run`) and
  `--isa native|sse|avx|avx2|avx512` (instruction set of `build`).
