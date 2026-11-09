// A measurement, not a part of the language nor of its runtime (D594): the
// fast paths of the small blocks of the allocator (alloc_cached, free_cached
// and the entries isc_alloc, isc_free of crates/isc-runtime) written several
// ways, the same algorithm, to price the model of the proposal
// docs/proposals/runtime-kernel.md before anything of it is built:
//   ptr:  addresses, as the runtime does today
//   h64:  handles of 64 bits (byte offsets in the arena), masked into the
//         arena before each use
//   h64c: the same handles, checked to be in the arena (no mask)
//   h64p: the handle is the address itself, checked to be in the arena
//   h32:  handles of 32 bits counted in 64 bytes (an arena of 256 GiB)
// each one inlined into its caller (what code of the prelude would be) and,
// with the suffix `.called`, called as the functions of the runtime are today.
//
//   rustc --edition 2021 -C opt-level=3 -C target-cpu=x86-64-v3 -C panic=abort \
//       -o arena_handles bench/prototypes/arena_handles.rs
//   ./arena_handles pair.h64p 41000000     (pair|batch).(ptr|h64|h64c|h64p|h32)[.called]
//
// Cycles and instructions an operation: `perf stat` of two round counts, their
// difference divided by the operations between them (2 a round of `pair`, 128
// a round of `batch`).
use std::cell::UnsafeCell;
use std::hint::black_box;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering::Relaxed};

const HEADER: usize = 64;
const CLASSES: usize = 80;
const IN_USE: u8 = 0xA5;
const FREE: u8 = 0x5A;
const DIRTY: u8 = 0;
const ARENA: usize = 1 << 40;
const MASK: usize = ARENA - 1;
const ARENA32: usize = 1 << 38;

static HEAP_MODE: AtomicU8 = AtomicU8::new(2);
static BASE: AtomicUsize = AtomicUsize::new(0);
static BUMP: AtomicUsize = AtomicUsize::new(0);
static LIMIT: [u32; CLASSES] = [256; CLASSES];

#[repr(C, align(64))]
struct Empty([u8; 64]);
static EMPTY: Empty = Empty([0; 64]);

#[repr(C, align(64))]
struct Hdr {
    class: u32,
    state: u8,
    contents: u8,
    idle: u8,
    pad: u8,
    next: usize, // an address (ptr), a handle (h64), or a handle in its low 32 bits (h32)
    rest: [u64; 6],
}

#[repr(C)]
struct Cache {
    heads: [usize; CLASSES],
    counts: [u16; CLASSES],
    registered: u32,
}

thread_local! {
    static TC: UnsafeCell<Cache> = const { UnsafeCell::new(Cache { heads: [0; CLASSES], counts: [0; CLASSES], registered: 1 }) };
}

#[inline(always)]
fn tc() -> *mut Cache {
    TC.with(|c| c.get())
}

/// A new block of the class from the arena (the slow path: not measured).
#[cold]
#[inline(never)]
unsafe fn carve(class: usize) -> *mut Hdr {
    let size = (class + 1) * 64 + HEADER;
    let at = BUMP.fetch_add(size, Relaxed);
    let h = (BASE.load(Relaxed) + at) as *mut Hdr;
    (*h).class = class as u32;
    (*h).state = IN_USE;
    h
}

#[cold]
#[inline(never)]
unsafe fn free_cold(_p: *mut u8) {}

// ---- ptr -----------------------------------------------------------------
#[inline(always)]
unsafe fn alloc_ptr(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return EMPTY.0.as_ptr() as *mut u8;
    }
    let class = (bytes as usize + 63) / 64 - 1;
    if HEAP_MODE.load(Relaxed) == 2 && bytes <= 1024 {
        let tc = tc();
        let h = (*tc).heads[class] as *mut Hdr;
        if !h.is_null() && (*tc).registered != 0 && (*h).state == FREE {
            (*tc).heads[class] = (*h).next;
            (*tc).counts[class] -= 1;
            (*h).state = IN_USE;
            (*h).contents = DIRTY;
            return (h as *mut u8).add(HEADER);
        }
    }
    (carve(class) as *mut u8).add(HEADER)
}

#[inline(always)]
unsafe fn free_ptr(p: *mut u8) {
    if p == EMPTY.0.as_ptr() as *mut u8 || p.is_null() {
        return;
    }
    if HEAP_MODE.load(Relaxed) == 2 && p as usize % 64 == 0 {
        let h = p.sub(HEADER) as *mut Hdr;
        let class = (*h).class as usize;
        if class < CLASSES && (*h).state == IN_USE {
            let tc = tc();
            if (*tc).registered != 0 && ((*tc).counts[class] as u32) < LIMIT[class] {
                let head = (*tc).heads[class];
                (*h).state = FREE;
                (*h).next = head;
                (*tc).heads[class] = h as usize;
                (*tc).counts[class] += 1;
                return;
            }
        }
    }
    free_cold(p)
}

// ---- h64 -----------------------------------------------------------------
#[inline(always)]
unsafe fn alloc_h64(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return EMPTY.0.as_ptr() as *mut u8;
    }
    let class = (bytes as usize + 63) / 64 - 1;
    if HEAP_MODE.load(Relaxed) == 2 && bytes <= 1024 {
        let tc = tc();
        let hd = (*tc).heads[class];
        if hd != 0 && (*tc).registered != 0 {
            // a handle of the allocator's data: forced into the arena
            let h = (BASE.load(Relaxed) + (hd & MASK)) as *mut Hdr;
            if (*h).state == FREE {
                (*tc).heads[class] = (*h).next;
                (*tc).counts[class] -= 1;
                (*h).state = IN_USE;
                (*h).contents = DIRTY;
                return (h as *mut u8).add(HEADER);
            }
        }
    }
    (carve(class) as *mut u8).add(HEADER)
}

#[inline(always)]
unsafe fn free_h64(p: *mut u8) {
    // an address of the program: in the arena (not null, not the empty block,
    // not a literal: one test), on 64 bytes
    let hd = (p as usize).wrapping_sub(BASE.load(Relaxed)).wrapping_sub(HEADER);
    if hd < ARENA - HEADER && hd % 64 == 0 && HEAP_MODE.load(Relaxed) == 2 {
        let h = p.sub(HEADER) as *mut Hdr;
        let class = (*h).class as usize;
        if class < CLASSES && (*h).state == IN_USE {
            let tc = tc();
            if (*tc).registered != 0 && ((*tc).counts[class] as u32) < LIMIT[class] {
                let head = (*tc).heads[class];
                (*h).state = FREE;
                (*h).next = head;
                (*tc).heads[class] = hd;
                (*tc).counts[class] += 1;
                return;
            }
        }
    }
    free_cold(p)
}

// ---- h64c: a handle checked to be in the arena (no mask) ---------------------
#[cold]
#[inline(never)]
unsafe fn alloc_slow_h64c(class: usize) -> *mut u8 {
    // (an empty list, or a handle out of the arena: a corrupted heap, a panic there)
    (carve(class) as *mut u8).add(HEADER)
}

#[inline(always)]
unsafe fn alloc_h64c(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return EMPTY.0.as_ptr() as *mut u8;
    }
    let class = (bytes as usize + 63) / 64 - 1;
    if HEAP_MODE.load(Relaxed) == 2 && bytes <= 1024 {
        let tc = tc();
        let hd = (*tc).heads[class];
        // not 0 and in the arena: one test (0 - 1 is above every handle)
        if hd.wrapping_sub(1) < ARENA - HEADER - 1 && (*tc).registered != 0 {
            let h = (BASE.load(Relaxed) + hd) as *mut Hdr;
            if (*h).state == FREE {
                (*tc).heads[class] = (*h).next;
                (*tc).counts[class] -= 1;
                (*h).state = IN_USE;
                (*h).contents = DIRTY;
                return (h as *mut u8).add(HEADER);
            }
        }
    }
    alloc_slow_h64c(class)
}

// ---- h64p: the handle is the address, checked to be in the arena before its use
#[inline(always)]
unsafe fn alloc_h64p(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return EMPTY.0.as_ptr() as *mut u8;
    }
    let class = (bytes as usize + 63) / 64 - 1;
    if HEAP_MODE.load(Relaxed) == 2 && bytes <= 1024 {
        let tc = tc();
        let hd = (*tc).heads[class];
        // in the arena (0, no block, is not): one test
        if hd.wrapping_sub(BASE.load(Relaxed)) < ARENA - HEADER && (*tc).registered != 0 {
            let h = hd as *mut Hdr;
            if (*h).state == FREE {
                (*tc).heads[class] = (*h).next;
                (*tc).counts[class] -= 1;
                (*h).state = IN_USE;
                (*h).contents = DIRTY;
                return (h as *mut u8).add(HEADER);
            }
        }
    }
    alloc_slow_h64c(class)
}

#[inline(always)]
unsafe fn free_h64p(p: *mut u8) {
    let hd = (p as usize).wrapping_sub(HEADER);
    if hd.wrapping_sub(BASE.load(Relaxed)) < ARENA - HEADER && hd % 64 == 0 && HEAP_MODE.load(Relaxed) == 2 {
        let h = hd as *mut Hdr;
        let class = (*h).class as usize;
        if class < CLASSES && (*h).state == IN_USE {
            let tc = tc();
            if (*tc).registered != 0 && ((*tc).counts[class] as u32) < LIMIT[class] {
                let head = (*tc).heads[class];
                (*h).state = FREE;
                (*h).next = head;
                (*tc).heads[class] = hd;
                (*tc).counts[class] += 1;
                return;
            }
        }
    }
    free_cold(p)
}

// ---- h32 -----------------------------------------------------------------
#[inline(always)]
unsafe fn alloc_h32(bytes: i64) -> *mut u8 {
    if bytes <= 0 {
        return EMPTY.0.as_ptr() as *mut u8;
    }
    let class = (bytes as usize + 63) / 64 - 1;
    if HEAP_MODE.load(Relaxed) == 2 && bytes <= 1024 {
        let tc = tc();
        let hd = (*tc).heads[class] as u32;
        if hd != 0 && (*tc).registered != 0 {
            // 32 bits counted in 64 bytes: always in the arena
            let h = (BASE.load(Relaxed) + ((hd as usize) << 6)) as *mut Hdr;
            if (*h).state == FREE {
                (*tc).heads[class] = (*h).next as u32 as usize;
                (*tc).counts[class] -= 1;
                (*h).state = IN_USE;
                (*h).contents = DIRTY;
                return (h as *mut u8).add(HEADER);
            }
        }
    }
    (carve(class) as *mut u8).add(HEADER)
}

#[inline(always)]
unsafe fn free_h32(p: *mut u8) {
    let off = (p as usize).wrapping_sub(BASE.load(Relaxed)).wrapping_sub(HEADER);
    if off < ARENA32 - HEADER && off % 64 == 0 && HEAP_MODE.load(Relaxed) == 2 {
        let h = p.sub(HEADER) as *mut Hdr;
        let class = (*h).class as usize;
        if class < CLASSES && (*h).state == IN_USE {
            let tc = tc();
            if (*tc).registered != 0 && ((*tc).counts[class] as u32) < LIMIT[class] {
                let head = (*tc).heads[class];
                (*h).state = FREE;
                (*h).next = head;
                (*tc).heads[class] = off >> 6;
                (*tc).counts[class] += 1;
                return;
            }
        }
    }
    free_cold(p)
}

// ---- called versions (as the runtime's functions are today) ---------------
macro_rules! called {
    ($a:ident, $f:ident, $ai:ident, $fi:ident) => {
        #[inline(never)]
        unsafe extern "C" fn $a(bytes: i64) -> *mut u8 {
            $ai(bytes)
        }
        #[inline(never)]
        unsafe extern "C" fn $f(p: *mut u8) {
            $fi(p)
        }
    };
}
called!(alloc_ptr_c, free_ptr_c, alloc_ptr, free_ptr);
called!(alloc_h64_c, free_h64_c, alloc_h64, free_h64);
called!(alloc_h32_c, free_h32_c, alloc_h32, free_h32);
called!(alloc_h64c_c, free_h64c_c, alloc_h64c, free_h64);

// ---- patterns --------------------------------------------------------------
macro_rules! patterns {
    ($pair:ident, $batch:ident, $a:ident, $f:ident) => {
        /// One block taken, written and given back, `rounds` times.
        #[inline(never)]
        unsafe fn $pair(rounds: u64) -> u64 {
            let mut sum = 0u64;
            for r in 0..rounds {
                let bytes = black_box(((r & 3) as i64 + 1) * 64);
                let p = $a(bytes);
                *(p as *mut u64) = r;
                sum = sum.wrapping_add(*(p as *const u64));
                $f(black_box(p));
            }
            sum
        }
        /// 64 blocks taken, then given back in the order they were taken.
        #[inline(never)]
        unsafe fn $batch(rounds: u64) -> u64 {
            let mut ps = [std::ptr::null_mut::<u8>(); 64];
            let mut sum = 0u64;
            for r in 0..rounds {
                for (i, slot) in ps.iter_mut().enumerate() {
                    let p = $a(black_box(((i & 3) as i64 + 1) * 64));
                    *(p as *mut u64) = r + i as u64;
                    *slot = p;
                }
                for slot in ps.iter() {
                    sum = sum.wrapping_add(*(*slot as *const u64));
                    $f(*slot);
                }
                black_box(&mut ps);
            }
            sum
        }
    };
}
patterns!(pair_ptr, batch_ptr, alloc_ptr, free_ptr);
patterns!(pair_h64, batch_h64, alloc_h64, free_h64);
patterns!(pair_h32, batch_h32, alloc_h32, free_h32);
patterns!(pair_ptr_c, batch_ptr_c, alloc_ptr_c, free_ptr_c);
patterns!(pair_h64_c, batch_h64_c, alloc_h64_c, free_h64_c);
patterns!(pair_h32_c, batch_h32_c, alloc_h32_c, free_h32_c);
patterns!(pair_h64c, batch_h64c, alloc_h64c, free_h64);
patterns!(pair_h64p, batch_h64p, alloc_h64p, free_h64p);
patterns!(pair_h64c_c, batch_h64c_c, alloc_h64c_c, free_h64c_c);

/// 1 TiB of addresses, none committed before its use.
unsafe fn reserve() -> usize {
    let p: usize;
    std::arch::asm!(
        "syscall",
        inlateout("rax") 9usize => p,
        in("rdi") 0usize,
        in("rsi") ARENA,
        in("rdx") 3usize,
        in("r10") 0x4022usize,
        in("r8") usize::MAX,
        in("r9") 0usize,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    assert!(p < usize::MAX - 4096, "the arena could not be reserved");
    p
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rounds: u64 = a[2].parse().unwrap();
    unsafe {
        BASE.store(reserve(), Relaxed);
        BUMP.store(1 << 21, Relaxed);
        let s = match a[1].as_str() {
            "pair.ptr" => pair_ptr(rounds),
            "pair.h64" => pair_h64(rounds),
            "pair.h32" => pair_h32(rounds),
            "batch.ptr" => batch_ptr(rounds),
            "batch.h64" => batch_h64(rounds),
            "batch.h32" => batch_h32(rounds),
            "pair.h64c" => pair_h64c(rounds),
            "batch.h64c" => batch_h64c(rounds),
            "pair.h64c.called" => pair_h64c_c(rounds),
            "batch.h64c.called" => batch_h64c_c(rounds),
            "pair.h64p" => pair_h64p(rounds),
            "batch.h64p" => batch_h64p(rounds),
            "pair.ptr.called" => pair_ptr_c(rounds),
            "pair.h64.called" => pair_h64_c(rounds),
            "pair.h32.called" => pair_h32_c(rounds),
            "batch.ptr.called" => batch_ptr_c(rounds),
            "batch.h64.called" => batch_h64_c(rounds),
            "batch.h32.called" => batch_h32_c(rounds),
            _ => panic!("unknown pattern"),
        };
        println!("{}", s);
    }
}
