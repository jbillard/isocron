use isc_syntax::ast::*;
use isc_syntax::parse;

const POC: [(&str, &str); 4] = [
    ("sum", include_str!("../../../examples/poc/sum.isc")),
    ("dot", include_str!("../../../examples/poc/dot.isc")),
    ("histogram", include_str!("../../../examples/poc/histogram.isc")),
    ("std_dev", include_str!("../../../examples/poc/std_dev.isc")),
];

#[test]
fn poc_programs_parse() {
    for (name, src) in POC {
        let p = parse(src, 0).unwrap_or_else(|e| panic!("{}: {}", name, e));
        let main = p.decls.iter().find_map(|d| match d {
            Decl::Fn(f) if f.name == "main" => Some(f),
            _ => None,
        });
        let main = main.unwrap_or_else(|| panic!("{}: no main", name));
        assert!(main.is_action);
        match &main.body.kind {
            ExprKind::Block(stmts, last) => {
                assert!(stmts.len() >= 4, "{}: {} statements", name, stmts.len());
                assert!(matches!(last.kind, ExprKind::Int { value: 0, .. }));
            }
            k => panic!("{}: body is not a block: {:?}", name, k),
        }
    }
}

const SPEC_EXAMPLES: &str = r#"// Simple statistics on a file of floats
@requireNoAlloc
function average(x: f64[n]): f64 {
    return sum(x) / f64(n);
}

function std_dev(x: f64[n]): f64 {
    const m = average(x);
    return sqrt(sum((x - m) ** 2) / f64(n));
}

function histogram(values: u8[n]): u32[256] {
    return fold(values, zeros(), (c, v) => requireUnique(c.with(v, c[v] + 1)));
}

action main(args: string[k]): i32 {
    const data = readF64(args[0]);
    print(std_dev(data));
    return 0;
}

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

type Option<T> = None | Some(T);

type Result<T, E> = Ok(T) | Err(E);

function window<T>(x: T[n], k: size): T[n - k + 1][k] {
    return x;
}

function sum<T extends Num>(x: T[n]): T {
    return reduce(x, (+), 0);
}

declare function puts(s: string): i32;
"#;

#[test]
fn spec_examples_parse() {
    let p = parse(SPEC_EXAMPLES, 0).unwrap_or_else(|e| panic!("{}", e));
    assert_eq!(p.decls.len(), 11);
    let center = match &p.decls[5] {
        Decl::Fn(f) => f,
        _ => panic!(),
    };
    assert_eq!(center.name, "center");
    assert_eq!(center.modifiers.len(), 1);
    match &center.body.kind {
        ExprKind::Block(stmts, last) => {
            assert_eq!(stmts.len(), 1);
            assert!(matches!(last.kind, ExprKind::Binary(BinOp::Sub, _, _)));
        }
        k => panic!("{:?}", k),
    }
}

fn expr_of(src: &str) -> Expr {
    let p = parse(&format!("function f(): i64 {{\n    return {};\n}}\n", src), 0).unwrap_or_else(|e| panic!("{}: {}", src, e));
    match p.decls.into_iter().next().unwrap() {
        Decl::Fn(f) => f.body,
        _ => panic!(),
    }
}

fn show(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int { value, neg, .. } => format!("{}{}", if *neg { "-" } else { "" }, value),
        ExprKind::Float { value, .. } => format!("{}", value),
        ExprKind::Var(v) => v.clone(),
        ExprKind::Binary(op, a, b) => format!("({} {} {})", show(a), op.text(), show(b)),
        ExprKind::Unary(UnOp::Neg, a) => format!("(neg {})", show(a)),
        ExprKind::Unary(UnOp::Not, a) => format!("(not {})", show(a)),
        ExprKind::Pow(a, b) => format!("({} ^ {})", show(a), show(b)),
        ExprKind::Call(f, args) => {
            format!("{}({})", show(f), args.iter().map(show).collect::<Vec<_>>().join(", "))
        }
        ExprKind::Index(a, i) => format!("{}[{}]", show(a), show(i)),
        ExprKind::Slice(a, x, y) => format!("{}[{}..{}]", show(a), show(x), show(y)),
        ExprKind::Field(a, f) => format!("{}.{}", show(a), f),
        ExprKind::Lambda(ps, b) => {
            format!("(\\{} -> {})", ps.iter().map(|p| p.0.clone()).collect::<Vec<_>>().join(","), show(b))
        }
        ExprKind::OpFn(op) => format!("({})", op.text()),
        ExprKind::Tuple(es) => format!("<{}>", es.iter().map(show).collect::<Vec<_>>().join(", ")),
        ExprKind::Array(es) => format!("[{}]", es.iter().map(show).collect::<Vec<_>>().join(", ")),
        ExprKind::If(c, t, e) => format!("(if {} {} {})", show(c), show(t), show(e)),
        ExprKind::Update { target, access: Access::Index(i), value, require_unique } => {
            format!("({}{} with [{}] = {})", if *require_unique { "!" } else { "" }, show(target), show(i), show(value))
        }
        ExprKind::Update { target, access: Access::Field(f), value, .. } => {
            format!("({} with .{} = {})", show(target), f, show(value))
        }
        k => format!("{:?}", k),
    }
}

#[test]
fn precedence() {
    let cases = [
        ("1 + 2 * 3", "(1 + (2 * 3))"),
        ("1 - 2 - 3", "((1 - 2) - 3)"),
        ("-(x ** 2)", "(neg (x ^ 2))"),
        ("2 ** -1", "(2 ^ -1)"),
        ("a ** b ** c", "(a ^ (b ^ c))"),
        ("-5", "-5"),
        ("a || b && c", "(a || (b && c))"),
        ("!a == b", "((not a) == b)"),
        ("a === b", "(a == b)"),
        ("a < b + 1", "(a < (b + 1))"),
        // the bitwise operators below the comparisons, as in JavaScript
        ("a & b == c", "(a & (b == c))"),
        ("a | b ^ c & d", "(a | (b ^ (c & d)))"),
        ("a << 1 + 2", "(a << (1 + 2))"),
        ("c.with(v, c[v] + 1)", "(c with [v] = (c[v] + 1))"),
        ("requireUnique(c.with(v, 1))", "(!c with [v] = 1)"),
        ("{ ...r, a: 1 }", "(r with .a = 1)"),
        ("(a, b) => a + b", "(\\a,b -> (a + b))"),
        ("() => 1", "(\\ -> 1)"),
        ("x => x * 2", "(\\x -> (x * 2))"),
        ("(a + b) * c", "((a + b) * c)"),
        ("[1, 2] as const", "<1, 2>"),
        ("[1, 2]", "[1, 2]"),
        ("reduce(x, (+), 0)", "reduce(x, (+), 0)"),
        ("a ? b : c + 1", "(if a b (c + 1))"),
        ("a ? b : c ? d : e", "(if a b (if c d e))"),
        ("f(x)(y)", "f(x)(y)"),
        ("x.length", "len(x)"),
        ("x.slice(1, 2)", "x[1..2]"),
        ("xs.map(f)", "xs.map(f)"),
    ];
    for (src, want) in cases {
        assert_eq!(show(&expr_of(src)), want, "{}", src);
    }
}

/// `if` statements: a guard with `return` is the `if` of the value, the rest
/// of the body its other branch (D148).
#[test]
fn statements_are_lowered_to_expressions() {
    let src = "function f(x: i64): i64 {\n    if (x < 0) return 0;\n    const y = x * 2;\n    return y;\n}\n";
    let p = parse(src, 0).unwrap();
    let Decl::Fn(f) = &p.decls[0] else { panic!() };
    match &f.body.kind {
        ExprKind::If(_, t, e) => {
            assert_eq!(show(t), "0");
            assert!(matches!(e.kind, ExprKind::Block(..)));
        }
        k => panic!("{:?}", k),
    }
}

#[test]
fn bodies_and_declarations() {
    // `const` statements, then the value given by `return`
    let src = "function f(x: i64): i64 {\n    const a = x + 1;\n    const b = g(a, 2);\n    return a * b;\n}\n";
    let p = parse(src, 0).unwrap();
    let f = match &p.decls[0] {
        Decl::Fn(f) => f,
        _ => panic!(),
    };
    match &f.body.kind {
        ExprKind::Block(stmts, last) => {
            assert_eq!(stmts.len(), 2);
            assert_eq!(show(last), "(a * b)");
        }
        k => panic!("{:?}", k),
    }
    // two declarations
    let p = parse("function a(): i64 {\n    return 1;\n}\n\nfunction b(): i64 {\n    return 2;\n}\n", 0).unwrap();
    assert_eq!(p.decls.len(), 2);
}

#[test]
fn errors_have_positions() {
    let cases: [(&str, u32, u32); 10] = [
        ("function f(): i64 {\n    return (1 + ;\n}\n", 2, 17),
        ("function f(): i64 {\n    const a = 1;\n}\n", 1, 19),
        ("function f(): i64 {\n    return a < b < c;\n}\n", 2, 18),
        ("function f( : i64 {\n", 1, 13),
        ("function f(): i64 {\n    return 1 2;\n}\n", 2, 14),
        ("action main(): i32 {\n    const x = );\n    return 0;\n}\n", 2, 15),
        ("function f(): i64 {\n    let x = 1;\n    return x;\n}\n", 2, 5),
        ("function f(x: f64): f64 {\n    return -x ** 2;\n}\n", 2, 15),
        // a value is given by `return`, in an action too (D149)
        ("action f(): i64 {\n    print(1);\n}\n", 1, 8),
        ("function f(x: bool): i64 {\n    return 1;\n}\n", 1, 15),
    ];
    for (src, line, col) in cases {
        let e = parse(src, 0).expect_err(src);
        assert_eq!((e.span.line, e.span.col), (line, col), "{}: {}", src.escape_debug(), e);
    }
}

#[test]
fn match_parses() {
    let src = "function f(o: Option<i64>): i64 {\n    return match (o) {\n        None => 0,\n        Some(x) => x + 1,\n    };\n}\n";
    let p = parse(src, 0).unwrap();
    match &p.decls[0] {
        Decl::Fn(f) => match &f.body.kind {
            ExprKind::Match(_, arms) => assert_eq!(arms.len(), 2),
            k => panic!("{:?}", k),
        },
        _ => panic!(),
    }
}
