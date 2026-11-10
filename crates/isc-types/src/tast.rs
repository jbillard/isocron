//! Typed AST produced by the checker. Every expression carries its type.
//! Types may mention the type parameters (`Ty::Param`) and size symbols of
//! the enclosing function; they never contain inference variables.

use crate::types::{IntKind, Poly, SymId, Ty};
use isc_syntax::ast::{BinOp, Modifier, UnOp};
use isc_syntax::Span;

pub type FnId = usize;
pub type LocalId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarTy {
    Int(IntKind),
    F64,
    F32,
}

impl ScalarTy {
    pub fn to_ty(self) -> Ty {
        match self {
            ScalarTy::Int(k) => Ty::Int(k),
            ScalarTy::F64 => Ty::F64,
            ScalarTy::F32 => Ty::F32,
        }
    }
}

/// The actions with a fallible form `<name>OrError` (D138, D151): the value
/// and an error message, `""` when the action succeeds, instead of a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fallible {
    ParseF64,
    ReadBytes,
    ReadF64,
    ReadText,
    ReadCsvF64,
    WriteBytes,
    WriteText,
}

impl Fallible {
    pub fn from_name(name: &str) -> Option<Fallible> {
        Some(match name.strip_suffix("OrError")? {
            "parseF64" => Fallible::ParseF64,
            "readBytes" => Fallible::ReadBytes,
            "readF64" => Fallible::ReadF64,
            "readText" => Fallible::ReadText,
            "readCsvF64" => Fallible::ReadCsvF64,
            "writeBytes" => Fallible::WriteBytes,
            "writeText" => Fallible::WriteText,
            _ => return None,
        })
    }
}

/// Built-in primitives (SPEC sections 6 and 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prim {
    Map,
    /// `mapParallel(x, f)` (D373): the action `f` applied to each element,
    /// the applications at the same time on the threads of the pool.
    MapParallel,
    /// `isPure(f)` (D386): whether the function `f` is pure, a constant of
    /// each call compiled; in the first branch of `if (isPure(h))`, `h` is
    /// a pure function.
    IsPure,
    /// `scan(x, op, e)`: the inclusive cumulation, from left to right.
    Scan,
    /// `filter(x, p)`: the elements where `p` is true, in order.
    Filter,
    /// `window(x, k)`: the sliding windows of `k` elements, a view.
    Window,
    /// `zip(a, b)`: the pairs of two arrays of the same size (section 6).
    Zip,
    /// `min(a, b)` of two numbers, or `min(x)` of an array (sections 6, 10).
    Min,
    Max,
    Reduce,
    Fold,
    Zeros,
    Iota,
    Fill,
    Sqrt,
    /// `exp`, `log`, `sin`, `cos`, `pow`, `fma` (SPEC section 10, D108).
    Math(MathFn),
    Convert(ScalarTy),
    ReadF64,
    ReadF32,
    ReadBytes,
    WriteF64,
    WriteF32,
    WriteBytes,
    Print,
    Clock,
    Bench,
    /// Strings (D134): `concat(a, b)` (also `a ++ b`), `len(s)` (bytes),
    /// `bytes(s)`, `stringOf(b)` (UTF-8 checked), `toString(v)` of a
    /// scalar, and the action `parseF64(s)`.
    Concat,
    StrLen,
    Bytes,
    StringOf,
    ToString,
    ParseF64,
    /// Text (D135): the actions `readText`, `writeText`,
    /// `parseCsvF64(text, skip)`, `readCsvF64(path, skip)`; the
    /// functions `lines`, `split`, `join` (`trim` is of the prelude, D590).
    ReadText,
    WriteText,
    ParseCsvF64,
    ReadCsvF64,
    Lines,
    Split,
    Join,
    /// `printBytes(x)`: the bytes of `x` to the standard output (D136).
    PrintBytes,
    /// A fallible form of an action (D138).
    OrError(Fallible),
    /// The functions of strings of TypeScript (D155), on bytes of UTF-8
    /// (`indexOf` and `includes`: functions of the prelude, D596).
    LastIndexOf,
    StartsWith,
    EndsWith,
    ReplaceAll,
    ToUpper,
    ToLower,
    PadStart,
    PadEnd,
    Repeat,
    /// `x.toFixed(d)`: `d` decimals, as JavaScript.
    ToFixed,
    /// `s.slice(a, b)` of a string.
    StrSlice,
    /// The standard input (D155): its bytes, its text, its lines.
    ReadStdin,
    ReadStdinText,
    ReadStdinLines,
    /// `$group(keys)` (D167): the first, last and count of each distinct key
    Group,
    /// `$groupCounts(keys)` (D317): `$group` without the last indices
    /// (zeros), for `countBy`
    GroupCounts,
    /// `$hashIndex(keys)` (D492): the index of hashes of the distinct keys of
    /// a map
    HashIndex,
    /// `$hashFind(keys, index, k)` (D492): the position of `k` among the
    /// keys, -1 if absent
    HashFind,
    /// `readWord(b, at, n)` (D580): the `n` bytes (1 to 8) of `b` from
    /// `at`, little-endian, as a `u64`, checked once
    ReadWord,
    /// `$panic(a, b, c)` (D582): a panic of the prelude, its message the
    /// three texts, of any type (the one its place expects); the prelude
    /// alone can name it
    Panic,
    /// `leadingZeros(x)` (D588): the zero bits above the highest one of an
    /// integer, in the width of its type (the width for 0), as an `i64`
    LeadingZeros,
    /// `$textTail(w0, w1, w2, n, minus)` (D588): the last `n` bytes (1 to
    /// 24) of the 24 bytes of three words (the first at the lowest place),
    /// a `-` before them when `minus`, a string; every byte of the words
    /// ASCII (a panic otherwise); the prelude alone can name it
    TextTail,
    /// `$floatBits(x)`: the bits of a `f64`, or of a `f32` (in the low
    /// half), as a `u64`; a NaN has the bits its target gave it, so the
    /// prelude alone can name it (the text of a float reads no bit of a
    /// NaN but that it is one)
    FloatBits,
    /// `$mulHigh(a, b)`: the high 64 bits of the product of two `u64` (128
    /// bits); the prelude alone can name it
    MulHigh,
    /// `$decimalText(d, w1, w2, n, point, minus)`: a decimal number of 17
    /// digits as a string, without exponent: the first digit `d` (0 to 9),
    /// the 16 next the bytes of the words `w1` and `w2` (ASCII, the first
    /// at the lowest place), the first `n` of the 17 written (1 to 17: the
    /// others are zeros), `point` digits before the point (zeros added
    /// after the digits when there are fewer, `0.` and zeros before them
    /// when `point` is 0 or less), a `-` first when `minus`; a panic for a
    /// `d`, an `n` or a `point` out of their bounds or a byte not ASCII;
    /// the prelude alone can name it
    DecimalText,
    /// `$withText(t, at, piece)`: the text `t` whose bytes from `at` are
    /// those of the text `piece` (as many of them replaced); a panic unless
    /// they lie in `t` and both their ends are between two characters of
    /// `t`, so that the result is a text; `t` given at its last use is
    /// written where it is; the prelude alone can name it
    WithText,
    /// `$words("...")`: the array of the `u64` whose hexadecimal digits the
    /// literal holds, 16 a word, its size known to the types: data of the
    /// program written as one literal (a table of the prelude costs each
    /// compilation its tokens: one, where an array literal has one a word);
    /// the prelude alone can name it
    Words,
    /// `$fixedText(i0, i1, i2, ni, f0, f1, f2, nf, minus)`: a decimal
    /// number without exponent as a string, its digits given as bytes: the
    /// last `ni` bytes (1 to 24) of the 24 bytes of the words `i0`, `i1`,
    /// `i2` (ASCII, the first at the lowest place), then, when `nf` is not
    /// 0, a point and the last `nf` bytes (1 to 24) of the words `f0`,
    /// `f1`, `f2`; a `-` first when `minus`; a panic for an `ni` or an `nf`
    /// out of their bounds or a byte not ASCII; the prelude alone can name
    /// it
    FixedText,
    /// A primitive of the system (D322), called by the standard modules only.
    Sys(SysOp),
    /// A primitive of the kernel of the runtime (D601), named by the
    /// prelude alone.
    Kernel(KernelOp),
    /// `argsort(x)`: the indices of the elements in their order, stable
    /// (D155): the primitive of `sort` and `sortBy`.
    ArgSort,
    /// `Number(s)`, `parseFloat(s)`, `parseInt(s, radix)` of ECMAScript
    /// (D156): a `f64`, NaN for a text that is not a number.
    JsNumber,
    ParseFloat,
    ParseInt,
    /// `BigInt(x)` of an integer or of a text (D156).
    ToBigInt,
    /// Regular expressions (D157): `RegExp(source, flags)`, `re.test(s)`,
    /// `s.search(re)`, `countMatches(s, re)`, `matches(s, re)`, and
    /// `s.replace(p, to)` of a text or a `RegExp`.
    /// `Map<K, V>` (D158), for the prelude: `$mapOf(keys, values, order)`
    /// and `$mapParts(m)`, its three arrays (the keys in the order of their
    /// first insertion, their values, the positions of the keys sorted)
    MapOf,
    MapParts,
    /// `m.size`
    MapSize,
    RegExpNew,
    RxTest,
    RxSearch,
    RxCount,
    RxMatches,
    Replace,
}

/// The mathematical functions of the runtime (SPEC section 10): one
/// implementation for every target and for the interpreter (D108); `fma`
/// rounds once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathFn {
    Exp,
    Log,
    Sin,
    Cos,
    Pow,
    Fma,
    // D155: those of TypeScript's `Math` (`round` as JavaScript rounds:
    // half toward +inf), and `abs` of a float
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Floor,
    Ceil,
    Trunc,
    Round,
    Abs,
}

impl MathFn {
    pub fn arity(self) -> usize {
        match self {
            MathFn::Pow | MathFn::Atan2 => 2,
            MathFn::Fma => 3,
            _ => 1,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            MathFn::Exp => "exp",
            MathFn::Log => "log",
            MathFn::Sin => "sin",
            MathFn::Cos => "cos",
            MathFn::Pow => "pow",
            MathFn::Fma => "fma",
            MathFn::Tan => "tan",
            MathFn::Asin => "asin",
            MathFn::Acos => "acos",
            MathFn::Atan => "atan",
            MathFn::Atan2 => "atan2",
            MathFn::Floor => "floor",
            MathFn::Ceil => "ceil",
            MathFn::Trunc => "trunc",
            MathFn::Round => "round",
            MathFn::Abs => "abs",
        }
    }
}

/// The primitives of the system (SPEC section 18, D322): actions of the
/// runtime that the modules of the standard library call, and they alone.
/// Each returns its value and the code of its error (0: none; 1 to 6 the
/// kinds of `IoError`; below 0 the error of the system, negated), the same
/// in the interpreter and in the compiled code.
/// The primitives of the kernel of the runtime written in Isocron (SPEC
/// section 17, D599, D601): the raw memory of the arena, named by the
/// address of a byte and tested to be inside it before each use, and the
/// zone of the thread. Actions that only the prelude can name (`$…`); the
/// same in the interpreter, over an arena and a zone of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KernelOp {
    /// `$arenaReserve() => base`: the one range of addresses of the heap,
    /// `KERNEL_ARENA` bytes, none of its pages usable yet; 0 when the
    /// system has none.
    ArenaReserve,
    /// `$arenaCommit(a, bytes) => code`: the pages of `bytes` bytes from
    /// `a`, inside the arena, readable and writable (zeros); 0, or the
    /// error of the system, negated.
    ArenaCommit,
    /// `$arenaBase()`, `$arenaSize()`: the arena reserved (0 before).
    ArenaBase,
    ArenaSize,
    /// `$arenaHas(a, bytes)`: whether the `bytes` bytes at `a` are inside
    /// the arena (never before it is reserved). The size of the arena is a
    /// constant: after this test, the generated code knows that the words
    /// of those bytes are inside, and tests none of them again (D605).
    ArenaHas,
    /// `$runtimeAlloc(bytes)`: a block of the allocator of the runtime,
    /// for what the kernel does not allocate (transitional: while that
    /// allocator exists, D605). A panic when interpreted.
    RuntimeAlloc,
    /// `$load8(a)`, `$load16`, `$load32`, `$load64`: the bytes at `a` as an
    /// unsigned word; a panic when they are not inside the arena.
    Load(u8),
    /// `$store8(a, v)`, `$store16`, `$store32`, `$store64`: the low bytes of
    /// `v` written at `a`; a panic when they are not inside the arena.
    Store(u8),
    /// `$zoneLoad16(at)`, `$zoneLoad32`, `$zoneLoad64`: the bytes at `at` of
    /// the zone of the calling thread (zeros at its start); a panic past
    /// its end.
    ZoneLoad(u8),
    /// `$zoneStore16(at, v)`, `$zoneStore32`, `$zoneStore64`.
    ZoneStore(u8),
    /// `$cas64(a, expected, v) => before`: the word of 8 bytes at `a` (in
    /// the arena, on 8 bytes) becomes `v` if it is `expected`, in one step
    /// that no other thread can cut; the word it held before. The base of
    /// the locks of the kernel.
    Cas,
    /// `$atomicLoad64(a)`, `$atomicStore64(a, v)`: the word at `a` (in the
    /// arena, on 8 bytes) read or written in one step, in the order of the
    /// other atomic operations on it.
    AtomicLoad,
    AtomicStore,
    /// `$spinHint()`: the thread waits for another one a moment (`pause`).
    SpinHint,
}

/// The bytes of the zone of a thread (`KernelOp::ZoneLoad`).
pub const KERNEL_ZONE: u64 = 4096;
/// The bytes of the arena (`KernelOp::ArenaReserve`): addresses only, 1 TiB;
/// a constant, so that a test of a range of the arena is one comparison
/// and the tests of the words inside that range fold (D605).
pub const KERNEL_ARENA: u64 = 1 << 40;

impl KernelOp {
    /// The primitive of a name of the prelude.
    pub fn named(name: &str) -> Option<KernelOp> {
        Some(match name {
            "$arenaReserve" => KernelOp::ArenaReserve,
            "$arenaCommit" => KernelOp::ArenaCommit,
            "$arenaBase" => KernelOp::ArenaBase,
            "$arenaSize" => KernelOp::ArenaSize,
            "$arenaHas" => KernelOp::ArenaHas,
            "$runtimeAlloc" => KernelOp::RuntimeAlloc,
            "$load8" => KernelOp::Load(1),
            "$load16" => KernelOp::Load(2),
            "$load32" => KernelOp::Load(4),
            "$load64" => KernelOp::Load(8),
            "$store8" => KernelOp::Store(1),
            "$store16" => KernelOp::Store(2),
            "$store32" => KernelOp::Store(4),
            "$store64" => KernelOp::Store(8),
            "$zoneLoad16" => KernelOp::ZoneLoad(2),
            "$zoneLoad32" => KernelOp::ZoneLoad(4),
            "$zoneLoad64" => KernelOp::ZoneLoad(8),
            "$zoneStore16" => KernelOp::ZoneStore(2),
            "$zoneStore32" => KernelOp::ZoneStore(4),
            "$zoneStore64" => KernelOp::ZoneStore(8),
            "$cas64" => KernelOp::Cas,
            "$atomicLoad64" => KernelOp::AtomicLoad,
            "$atomicStore64" => KernelOp::AtomicStore,
            "$spinHint" => KernelOp::SpinHint,
            _ => return None,
        })
    }

    pub fn arity(self) -> usize {
        match self {
            KernelOp::ArenaReserve | KernelOp::ArenaBase | KernelOp::ArenaSize | KernelOp::SpinHint => 0,
            KernelOp::RuntimeAlloc | KernelOp::Load(_) | KernelOp::ZoneLoad(_) | KernelOp::AtomicLoad => 1,
            KernelOp::ArenaHas | KernelOp::ArenaCommit | KernelOp::Store(_) | KernelOp::ZoneStore(_) | KernelOp::AtomicStore => 2,
            KernelOp::Cas => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SysOp {
    /// `sysGetenv(name) => [value, found]`
    Getenv,
    /// `sysEnviron() => string[k]`: the entries `name=value`, in the order
    /// of their bytes
    Environ,
    /// `sysStat(path) => [kind, size, modified, code]`: kind 1 a file, 2 a
    /// folder, 3 something else; modified in nanoseconds since 1970
    Stat,
    /// `sysReadDir(path) => [names, code]`: without `.` and `..`, in the
    /// order of their bytes
    ReadDir,
    /// `sysMkdir(path) => code`
    Mkdir,
    /// `sysRemoveFile(path) => code`
    RemoveFile,
    /// `sysRemoveDir(path) => code`: an empty folder
    RemoveDir,
    /// `sysRename(from, to) => code`
    Rename,
    /// `sysCwd() => [path, code]`
    Cwd,
    /// `sysReadFile(path) => [bytes, code]`
    ReadFile,
    /// `sysWriteFile(path, bytes, append) => code`
    WriteFile,
    /// `sysNow() => nanoseconds`: the time of the clock of the system since
    /// 1970-01-01T00:00:00Z
    Now,
    /// `sysConnect(address, port) => [conn, code]`: a numeric address (D339)
    Connect,
    /// `sysListen(address, port) => [listener, code]`: port 0 for any
    Listen,
    /// `sysAccept(listener) => [conn, peer, code]`
    Accept,
    /// `sysRecv(conn, max) => [bytes, code]`: none at the end of the stream
    Recv,
    /// `sysSend(conn, bytes) => code`: all of them, or an error
    Send,
    /// `sysClose(conn) => code`: consumes the connection
    Close,
    /// `sysCloseListener(listener) => code`: consumes the listener
    CloseListener,
    /// `sysSetTimeout(conn, ms) => code`: of each read and write (0: none)
    SetTimeout,
    /// `sysLocalPort(listener) => port`
    LocalPort,
    /// `sysUdpQuery(address, port, packet, ms) => [answer, code]`: one
    /// datagram sent, one received (the resolver of names, D326)
    UdpQuery,
    /// `sysPlatform() => 0` Linux, `1` Windows, `2` WASI
    Platform,
    /// `sysThreads() => n`: the threads of the pool (D373; 1 in the
    /// interpreter and under WASI)
    Threads,
    /// `sysListenShared(address, port) => [listener, code]`: a listener that
    /// shares its port with the other ones of the program (`SO_REUSEPORT`,
    /// D374: the kernel shares the connections out); Linux only
    ListenShared,
    /// `sysResolve(host) => [addresses, code]`: the resolver of Windows
    /// (Linux: the module resolves, code 7)
    Resolve,
    /// `sysShutdown(conn) => code`: its sending side closed (the peer reads
    /// the end of the stream), the connection kept
    Shutdown,
    /// `sysServerOpen(listener) => [server, code]` (D342): the listener
    /// taken by a server, which holds its connections by number
    ServerOpen,
    /// `sysServerWait(server, max, ms) => [ids, lens, bytes, code]`: the
    /// connections waiting accepted, then at most `max` bytes read from
    /// each connection that has some (0: its end), within `ms` ms
    ServerWait,
    /// `sysServerSend(server, id, bytes) => code`: all of them
    ServerSend,
    /// `sysServerDrop(server, id) => code`: the connection closed
    ServerDrop,
    /// `sysServerClose(server) => code`: all closed
    ServerClose,
    /// `sysTlsStart(conn, host, ca) => [text, code]` (D343): the client of
    /// TLS on the connection, its reads and writes encrypted from then on
    TlsStart,
    /// `sysServerOpenTls(listener, cert, key) => [server, text, code]`: a
    /// server whose connections speak TLS
    ServerOpenTls,
}

impl SysOp {
    pub const ALL: [SysOp; 34] = [
        SysOp::Getenv,
        SysOp::Environ,
        SysOp::Stat,
        SysOp::ReadDir,
        SysOp::Mkdir,
        SysOp::RemoveFile,
        SysOp::RemoveDir,
        SysOp::Rename,
        SysOp::Cwd,
        SysOp::ReadFile,
        SysOp::WriteFile,
        SysOp::Now,
        SysOp::Connect,
        SysOp::Listen,
        SysOp::Accept,
        SysOp::Recv,
        SysOp::Send,
        SysOp::Close,
        SysOp::CloseListener,
        SysOp::SetTimeout,
        SysOp::LocalPort,
        SysOp::UdpQuery,
        SysOp::Platform,
        SysOp::Threads,
        SysOp::ListenShared,
        SysOp::Resolve,
        SysOp::Shutdown,
        SysOp::ServerOpen,
        SysOp::ServerWait,
        SysOp::ServerSend,
        SysOp::ServerDrop,
        SysOp::ServerClose,
        SysOp::TlsStart,
        SysOp::ServerOpenTls,
    ];

    /// It consumes its first argument (a resource of the system closed,
    /// D339).
    pub fn consumes(self) -> bool {
        matches!(self, SysOp::Close | SysOp::CloseListener | SysOp::ServerOpen | SysOp::ServerClose | SysOp::ServerOpenTls)
    }

    /// TLS (D343): the programs that reach it are linked with the C
    /// library of the system, which loads the library of TLS.
    pub fn is_tls(self) -> bool {
        matches!(self, SysOp::TlsStart | SysOp::ServerOpenTls)
    }

    /// The target WebAssembly (WASI preview 1) has it (D324): no sockets.
    pub fn on_wasi(self) -> bool {
        !matches!(
            self,
            SysOp::Connect
                | SysOp::Listen
                | SysOp::Accept
                | SysOp::Recv
                | SysOp::Send
                | SysOp::Close
                | SysOp::CloseListener
                | SysOp::SetTimeout
                | SysOp::LocalPort
                | SysOp::ListenShared
                | SysOp::UdpQuery
                | SysOp::Resolve
                | SysOp::Shutdown
                | SysOp::ServerOpen
                | SysOp::ServerWait
                | SysOp::ServerSend
                | SysOp::ServerDrop
                | SysOp::ServerClose
                | SysOp::TlsStart
                | SysOp::ServerOpenTls
        )
    }

    /// Its name in the modules.
    pub fn name(self) -> &'static str {
        match self {
            SysOp::Getenv => "sysGetenv",
            SysOp::Environ => "sysEnviron",
            SysOp::Stat => "sysStat",
            SysOp::ReadDir => "sysReadDir",
            SysOp::Mkdir => "sysMkdir",
            SysOp::RemoveFile => "sysRemoveFile",
            SysOp::RemoveDir => "sysRemoveDir",
            SysOp::Rename => "sysRename",
            SysOp::Cwd => "sysCwd",
            SysOp::ReadFile => "sysReadFile",
            SysOp::WriteFile => "sysWriteFile",
            SysOp::Now => "sysNow",
            SysOp::Connect => "sysConnect",
            SysOp::Listen => "sysListen",
            SysOp::Accept => "sysAccept",
            SysOp::Recv => "sysRecv",
            SysOp::Send => "sysSend",
            SysOp::Close => "sysClose",
            SysOp::CloseListener => "sysCloseListener",
            SysOp::SetTimeout => "sysSetTimeout",
            SysOp::LocalPort => "sysLocalPort",
            SysOp::UdpQuery => "sysUdpQuery",
            SysOp::Platform => "sysPlatform",
            SysOp::Threads => "sysThreads",
            SysOp::ListenShared => "sysListenShared",
            SysOp::Resolve => "sysResolve",
            SysOp::Shutdown => "sysShutdown",
            SysOp::ServerOpen => "sysServerOpen",
            SysOp::ServerWait => "sysServerWait",
            SysOp::ServerSend => "sysServerSend",
            SysOp::ServerDrop => "sysServerDrop",
            SysOp::ServerClose => "sysServerClose",
            SysOp::TlsStart => "sysTlsStart",
            SysOp::ServerOpenTls => "sysServerOpenTls",
        }
    }
}

impl Prim {
    pub fn is_action(self) -> bool {
        matches!(
            self,
            Prim::Sys(_)
                | Prim::Kernel(_)
                | Prim::MapParallel
                | Prim::ReadF64
                | Prim::ReadF32
                | Prim::ReadBytes
                | Prim::WriteF64
                | Prim::WriteF32
                | Prim::WriteBytes
                | Prim::Print
                | Prim::Clock
                | Prim::Bench
                | Prim::ReadText
                | Prim::ReadStdin
                | Prim::ReadStdinText
                | Prim::ReadStdinLines
                | Prim::WriteText
                | Prim::ParseCsvF64
                | Prim::ReadCsvF64
                | Prim::PrintBytes
                // parsing a text has no effect: a function (D156)
                | Prim::OrError(
                    Fallible::ReadBytes
                        | Fallible::ReadF64
                        | Fallible::ReadText
                        | Fallible::ReadCsvF64
                        | Fallible::WriteBytes
                        | Fallible::WriteText
                )
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Int(i128),
    /// The value rounded to binary64 and, directly, to binary32.
    Float(f64, f32),
    Bool(bool),
    Unit,
    /// A string literal (D134).
    Str(String),
    /// A `bigint` literal (D156): its digits, their radix, its sign.
    BigInt(String, u32, bool),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TExpr {
    pub kind: TExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Callee {
    /// A top-level function, with its type arguments.
    Fn { id: FnId, type_args: Vec<Ty> },
    Prim(Prim),
    /// Any expression of function type (local, lambda, operator, ...).
    Value(Box<TExpr>),
    /// A C function declared with `extern` (`TProgram::externs`).
    Extern(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TExprKind {
    Lit(Lit),
    Local(LocalId),
    /// The value of a size expression, as an `i64`.
    SizeVal(Poly),
    /// A top-level function used as a value.
    FnRef { id: FnId, type_args: Vec<Ty> },
    /// A primitive used as a value (`sqrt`, conversions).
    PrimRef(Prim),
    /// An operator used as a value: `(+)`.
    OpRef(BinOp),
    /// A tuple, or a record (its fields in the order of their names).
    Tuple(Vec<TExpr>),
    Array(Vec<TExpr>),
    /// Field `index` of a record (fields in the order of their names).
    Field(Box<TExpr>, u32),
    /// `r with .f = v`: the record with field `index` replaced.
    FieldUpdate { target: Box<TExpr>, index: u32, value: Box<TExpr> },
    /// A value of a sum type: constructor `variant` of `sums[sum]`.
    Construct { sum: u32, variant: u32, args: Vec<TExpr> },
    /// `match`: the first arm whose pattern matches (the arms are exhaustive).
    Match(Box<TExpr>, Vec<(TPat, TExpr)>),
    /// `size_checks` lists the parameters whose array size could not be
    /// proved equal to the declared size at compile time: a run-time test is
    /// inserted for each (SPEC section 4).
    Call { callee: Callee, args: Vec<TExpr>, size_checks: Vec<usize> },
    Index(Box<TExpr>, Box<TExpr>),
    /// `x[a..b]` (SPEC section 6); its size is an existential symbol.
    Slice(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    Update { target: Box<TExpr>, index: Box<TExpr>, value: Box<TExpr>, require_unique: bool },
    Unary(UnOp, Box<TExpr>),
    /// `size_check` is set for an element-wise operation on two arrays whose
    /// sizes are not provably equal.
    Binary { op: BinOp, lhs: Box<TExpr>, rhs: Box<TExpr>, size_check: bool },
    Pow(Box<TExpr>, Box<TExpr>),
    If(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    Lambda(Vec<LocalId>, Box<TExpr>),
    Block(Vec<TStmt>, Box<TExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TStmt {
    Let(TPat, TExpr),
    Expr(TExpr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TPat {
    Wild(Ty),
    Bind(LocalId),
    /// A tuple, or a record (its fields in the order of their names).
    Tuple(Vec<TPat>, Ty),
    /// A literal (in `match`).
    Lit(Lit, Ty),
    /// A constructor of a sum type, with the patterns of its payload.
    Ctor { sum: u32, variant: u32, subs: Vec<TPat>, ty: Ty },
}

/// A sum type declared with `type Name<T, ...> = A | B(T, ...)`. In the
/// payload types, `Ty::Param(i)` is the i-th parameter of the type.
#[derive(Clone, Debug, PartialEq)]
pub struct SumDef {
    pub name: String,
    pub params: Vec<String>,
    pub variants: Vec<VariantDef>,
    /// The type contains itself, directly or through other types: its values
    /// are on the heap, with a reference count (SPEC section 8, D103).
    pub recursive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub payload: Vec<Ty>,
    /// The sizes its payload names (`Arr(Json[k])`, D335): their symbols 0
    /// to `sizes - 1` in `payload`, the sizes of each value of its own, any
    /// at a construction, new existential ones where a pattern takes it
    /// apart.
    pub sizes: usize,
}

impl SumDef {
    /// The payload types of a variant for the type arguments `args`.
    pub fn payload(&self, variant: usize, args: &[Ty]) -> Vec<Ty> {
        self.variants[variant].payload.iter().map(|t| subst_params(t, args)).collect()
    }
}

/// `t` with `Ty::Param(i)` replaced by `args[i]`.
pub fn subst_params(t: &Ty, args: &[Ty]) -> Ty {
    match t {
        Ty::Param(i) => args[*i as usize].clone(),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_params(t, args)).collect()),
        Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), subst_params(t, args))).collect()),
        Ty::Sum { id, name, args: a } => {
            Ty::Sum { id: *id, name: name.clone(), args: a.iter().map(|t| subst_params(t, args)).collect() }
        }
        Ty::Array(s, e) => Ty::Array(s.clone(), Box::new(subst_params(e, args))),
        Ty::Fn(ps, r, act) => Ty::Fn(ps.iter().map(|t| subst_params(t, args)).collect(), Box::new(subst_params(r, args)), *act),
        t => t.clone(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

/// The step of a `SymOrigin::ParamPath` to the elements of an array (D336).
pub const ELEMENT: u32 = u32::MAX;

/// The size bound by this path is a value of the function (its path does
/// not go through the elements of an array).
pub fn path_has_value(path: &[u32]) -> bool {
    !path.contains(&ELEMENT)
}

#[derive(Clone, Debug, PartialEq)]
pub enum SymOrigin {
    /// Bound by the size of the given (top-level array) parameter.
    Param(usize),
    /// Bound by the size of the array at this path in the given parameter:
    /// the indices of the fields of records (in their order of names) and of
    /// the elements of tuples (`r: { xs: f64[k] }`, D336); `ELEMENT` for the
    /// elements of an array (the rows of `t: f64[r][c]`): a size known to
    /// the types only, not a value of the function.
    ParamPath(usize, Vec<u32>),
    /// The value of the given `size` parameter (`k: size`).
    SizeParam(usize),
    /// The value of a local `i64` variable, given as a `size` argument.
    Local(LocalId),
    /// Fixed at run time by the primitive call at this position.
    Existential(Span),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SymInfo {
    pub name: String,
    pub origin: SymOrigin,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TFn {
    pub name: String,
    pub span: Span,
    pub is_action: bool,
    pub from_prelude: bool,
    pub generics: Vec<(String, crate::types::class::Classes)>,
    pub syms: Vec<SymInfo>,
    pub params: Vec<LocalId>,
    pub param_tys: Vec<Ty>,
    pub ret: Ty,
    pub locals: Vec<Local>,
    pub body: TExpr,
    /// The requirements of the declaration (SPEC section 9), checked on the
    /// IR (D106).
    pub requires: Vec<(Modifier, Span)>,
}

impl TFn {
    pub fn sym_name(&self, s: SymId) -> String {
        self.syms.get(s as usize).map(|i| i.name.clone()).unwrap_or_else(|| format!("?{}", s))
    }
}

/// A C function declared with `extern name(params) -> type` (System V
/// x86-64 ABI): scalars as C scalars, `string` as a NUL-terminated copy,
/// an array of scalars as a pointer to its elements (read only) followed by
/// its length (D125).
#[derive(Clone, Debug, PartialEq)]
pub struct TExtern {
    pub name: String,
    pub span: Span,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TProgram {
    pub fns: Vec<TFn>,
    pub main: FnId,
    pub sums: Vec<SumDef>,
    /// The C functions declared with `extern` (SPEC section 12, D125).
    pub externs: Vec<TExtern>,
    /// `recursive[f]`: `f` can call itself (it is not inlined, SPEC section
    /// 11).
    pub recursive: Vec<bool>,
}
