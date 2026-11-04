//! The allocator of Isocron (DECISIONS D79, D81), used by the compiled
//! programs on Linux; `ISC_ALLOC=glibc` uses glibc's `aligned_alloc` and
//! `free` instead. WebAssembly keeps the allocator of wasi-libc.
//!
//! Every block is preceded by a header of 64 bytes. Three kinds of blocks:
//!
//! - Small blocks (up to 256 KiB) belong to a size class
//!   (`isc_runtime_core::alloc_class`: multiples of 64 bytes up to 1 KiB,
//!   then 8 classes per power of two) and are carved from chunks of 4 MiB in
//!   huge pages. Each thread keeps freed small blocks in its own cache, in
//!   its thread-local storage: a block is taken from it or given back to it
//!   with a few instructions, no lock and no atomic operation. An empty cache
//!   takes several blocks at once from the global lists (one lock); a full
//!   one gives the block to the global lists. A thread that ends gives its
//!   cache back.
//! - Medium blocks (up to 32 MiB) are cut from regions of 256 MiB of address
//!   space (physical pages come when they are touched) by a two-level
//!   segregated fit (TLSF): a free block of the right size is found in
//!   constant time with two levels of bitmaps, the rest of it stays free,
//!   and a freed block is merged with its free neighbours, so that memory
//!   freed by blocks of one size serves blocks of another.
//! - Large blocks have their own mapping, in huge pages and "colored" (see
//!   `new_large`). Once freed they go to a cache, so that the temporary array
//!   of the next call of a kernel gets the same pages again, already mapped,
//!   where glibc maps and unmaps every block above 32 MiB and each call pays
//!   all its page faults again. The cache holds at most max(64 MiB, the
//!   bytes of the large blocks in use): beyond, the blocks freed the longest
//!   ago are unmapped, and so are blocks gone stale (see `trim_large`). A
//!   request that finds no cached block of its size grows the largest
//!   cached block smaller than it (`grow_cached`).
//!
//! Memory given back: the free medium blocks that still have written pages
//! are "idle". When the idle bytes exceed 64 MiB plus the medium and large
//! bytes in use, the blocks freed the longest ago give their pages back to
//! the system (`MADV_DONTNEED`) until the idle bytes are a quarter below
//! that bound; so do the blocks that stayed idle while the program allocated
//! more than twice that bound elsewhere. Only whole huge pages go back (see
//! `trim_idle`). A program that stops using its memory, or uses other sizes,
//! gives it back; one that reuses it keeps it. The free small blocks are
//! kept, like glibc's bins: their memory is at most the largest amount of
//! small blocks the program used at once.
//!
//! A block that was never written is zero (the kernel gives zeroed pages):
//! `alloc` does not clear it again, it only writes one byte per page (see
//! `alloc`). Freeing a block twice, or a pointer the
//! allocator did not give, is detected by the state of the header and is a
//! panic, instead of a silent corruption.

use core::ffi::{c_int, c_uint, c_void};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use isc_runtime_core::{alloc_class, alloc_class_size, Panic, ALLOC_CLASSES as CLASSES};

#[cfg(windows)]
use super::windows::{madvise, mmap, mremap, munmap, pthread_key_create, pthread_setspecific};

#[cfg(not(windows))]
use super::linux::{madvise, mmap, mremap, munmap};

// the end of a thread of glibc (the programs that declare C functions,
// D289): a key of pthread
#[cfg(not(windows))]
extern "C" {
    fn pthread_key_create(key: *mut c_uint, destructor: Option<unsafe extern "C" fn(*mut c_void)>) -> c_int;
    fn pthread_setspecific(key: c_uint, value: *const c_void) -> c_int;
}

const PROT_READ_WRITE: c_int = 0x1 | 0x2;
const MAP_PRIVATE_ANONYMOUS: c_int = 0x02 | 0x20;
const MAP_NORESERVE: c_int = 0x4000;
const MADV_DONTNEED: c_int = 4;
const MADV_HUGEPAGE: c_int = 14;
const MADV_NOHUGEPAGE: c_int = 15;

const HEADER: usize = 64;
const PAGE: usize = 4096;
const HUGE: usize = 2 << 20;

const SMALL_MAX: usize = 256 << 10;
/// The small classes are 0..SMALL_CLASSES (the last one is SMALL_MAX).
const SMALL_CLASSES: usize = 80;
const CHUNK: usize = 4 << 20;
/// Bytes of free blocks a thread keeps per small class (from 1 to
/// THREAD_BLOCKS_MAX blocks).
const THREAD_CLASS_BYTES: usize = 128 << 10;
/// The most blocks a thread keeps per class: half of it taken at each
/// refill, so that threads that make many small blocks at once take the lock
/// of their class rarely (D252: 32, the lines of a text made by 4 threads
/// spent 83 % of their time in `refill`).
const THREAD_BLOCKS_MAX: usize = 256;

const MEDIUM_MAX: usize = 32 << 20;
/// The class of the medium blocks.
const MEDIUM: u32 = u32::MAX - 1;
const REGION: usize = 256 << 20;
/// Medium blocks a thread keeps (two of them) are at most this large (an
/// array of 8 MiB and its header); a cached block serves a request at most a
/// quarter smaller.
const MEDIUM_CACHED: usize = (8 << 20) + PAGE;
/// A free medium block is cut when at least this much would be left.
const SPLIT_MIN: usize = 16 << 10;
const FL: usize = 48;

/// Idle memory always allowed (see the module documentation).
const IDLE_MIN: usize = 64 << 20;
/// The cache of large blocks may always hold this much.
const CACHE_MIN: usize = 64 << 20;
/// Colors of the large blocks: offsets 0, COLOR_STEP, ... (COLORS of them).
const COLOR_STEP: usize = (256 << 10) + 2048;
const COLORS: usize = 8;
/// Every large array starts LARGE_OFFSET + 64 bytes after a page boundary,
/// 1088 or 3136 with the colors, never a few lines after it: read from
/// memory, an array that starts 0 or 64 bytes after a page boundary is about
/// 3 % slower than one that starts 1024, 2112 or 3072 bytes after it, with
/// the same code (measured on xeon, one core, the fixed sum over 10^7
/// doubles; DECISIONS D83).
const LARGE_OFFSET: usize = 1024;

// PAGE_OFFSETS: every array of at least ALIGNED_MIN bytes starts 64 or 2112
// bytes after a page boundary (the medium blocks always 64), so that two
// arrays are 0 or 2048 bytes apart modulo 4096. Two arrays a few cache lines
// apart modulo 4096 are the worst case of "4K aliasing": in `y = x * 2.0`
// with y 64 bytes after x modulo 4096, the store to y[i] has the 12 low
// address bits of the next load, x[i + 8], which waits for it (DECISIONS
// D81). Blocks carved one after the other from a chunk were exactly that
// (their size plus the 64 bytes of a header).
const ALIGNED_MIN: usize = 16 << 10;

const _: () = assert!(match alloc_class(SMALL_MAX) {
    Some((c, _)) => c == SMALL_CLASSES - 1,
    None => false,
});

const THREAD_LIMIT: [u32; SMALL_CLASSES] = {
    let mut t = [0u32; SMALL_CLASSES];
    let mut i = 0;
    while i < SMALL_CLASSES {
        let n = THREAD_CLASS_BYTES / alloc_class_size(i);
        t[i] = if n < 1 {
            1
        } else if n > THREAD_BLOCKS_MAX {
            THREAD_BLOCKS_MAX as u32
        } else {
            n as u32
        };
        i += 1;
    }
    t
};

// states of a block
const IN_USE: u8 = 0xA5;
const FREE: u8 = 0x5A;
/// A medium block in the cache of a thread (in use for its neighbours).
const CACHED: u8 = 0xC3;
// contents of a free block
const DIRTY: u8 = 0;
/// Never written since it was mapped: all zero.
const ZERO: u8 = 1;
/// Its pages were given back (not all zero: its first page was kept).
const RELEASED: u8 = 2;

#[repr(C, align(64))]
struct Header {
    /// A small class, MEDIUM, or a large class.
    class: u32,
    state: u8,
    contents: u8,
    /// 1 when the block is in the idle list.
    idle: u8,
    _pad: u8,
    /// The free list of the block (small: singly linked).
    next: *mut Header,
    prev: *mut Header,
    /// The idle list (small and medium blocks) or the cache (large blocks),
    /// from the most recently freed to the least recently freed.
    newer: *mut Header,
    older: *mut Header,
    /// Medium blocks: the previous block of the region (null for the first).
    /// Large blocks: the start of the mapping.
    base: *mut Header,
    /// Medium blocks: the size of the block, header included. Large blocks:
    /// the length of the mapping.
    len: usize,
    /// Free medium blocks: an estimate of the pages written since they were
    /// mapped or given back (the idle bytes of the block). A block merged
    /// with the never-touched rest of a region must not count all of it: the
    /// release would then also take the pages in use at once.
    dirty: u32,
    /// Idle and cached blocks: `State::grown`, in MiB, when the block became
    /// idle or cached (its age).
    stamp: u32,
}

const _: () = assert!(core::mem::size_of::<Header>() == HEADER);

/// The cache of a thread, in its thread-local storage (`isc_heap_thread`).
#[repr(C)]
struct ThreadCache {
    heads: [*mut Header; SMALL_CLASSES],
    counts: [u32; SMALL_CLASSES],
    /// 1 once the end of the thread will give the cache back.
    registered: u32,
    /// The medium blocks of at most MEDIUM_CACHED bytes the thread freed
    /// last (state CACHED), the older one first.
    medium: [*mut Header; 2],
    /// The span of the thread (`span .. span_end`, whole huge pages of the
    /// chunks of the small blocks of less than ALIGNED_MIN bytes) that its
    /// new small blocks are carved from (D294).
    span: usize,
    span_end: usize,
}

const _: () = assert!(core::mem::size_of::<ThreadCache>() <= 1024);

// 1024 bytes of zeros in the static TLS block of every thread. The runtime
// is linked into the executable: the local-exec model applies, and the
// address of the cache is the thread pointer plus a constant. Under Windows
// (D246), the TLS section `.tls$` of the image, at an offset (`@SECREL32`)
// of the block of the thread (`gs:[0x58]` holds the blocks of the modules,
// `_tls_index` the one of the executable).
#[cfg(windows)]
core::arch::global_asm!(
    ".section .tls$,\"dw\"",
    ".p2align 6",
    ".globl isc_heap_thread",
    "isc_heap_thread:",
    ".skip 1024",
    ".text",
);

#[cfg(not(windows))]
core::arch::global_asm!(
    ".section .tbss,\"awT\",@nobits",
    ".p2align 6",
    ".globl isc_heap_thread",
    ".hidden isc_heap_thread",
    ".type isc_heap_thread,@object",
    ".size isc_heap_thread,1024",
    "isc_heap_thread:",
    ".skip 1024",
    ".text",
);

// The nodes of the recursive types (D166): cells of their exact size, a
// multiple of 16 bytes up to NODE_MAX, without a header, taken from and
// given back to a free list of the thread, carved from regions of
// NODE_REGION bytes. The size is known where the node is made and where it
// is freed (the compiled code), so nothing is stored with it. A cell freed
// by another thread joins the list of that thread. The regions are never
// given back: their memory is at most the largest number of nodes alive at
// once. `ISC_ALLOC=glibc` uses malloc and free for nodes too, so that
// Valgrind sees each of them.
pub const NODE_MAX: usize = 256;
const NODE_CLASSES: usize = NODE_MAX / 16;
const NODE_REGION: usize = 256 << 10;

#[repr(C)]
struct NodeCache {
    heads: [*mut u8; NODE_CLASSES],
    bump: *mut u8,
    end: *mut u8,
}

const _: () = assert!(core::mem::size_of::<NodeCache>() <= 256);

#[cfg(windows)]
core::arch::global_asm!(
    ".section .tls$,\"dw\"",
    ".p2align 6",
    ".globl isc_node_thread",
    "isc_node_thread:",
    ".skip 256",
    ".text",
);

#[cfg(not(windows))]
core::arch::global_asm!(
    ".section .tbss,\"awT\",@nobits",
    ".p2align 6",
    ".globl isc_node_thread",
    ".hidden isc_node_thread",
    ".type isc_node_thread,@object",
    ".size isc_node_thread,256",
    "isc_node_thread:",
    ".skip 256",
    ".text",
);

#[cfg(windows)]
#[inline(always)]
unsafe fn node_cache() -> *mut NodeCache {
    let p: *mut NodeCache;
    core::arch::asm!(
        "movq %gs:0x58, {0}",
        "movl _tls_index(%rip), {1:e}",
        "movq ({0},{1},8), {0}",
        "leaq isc_node_thread@SECREL32({0}), {0}",
        out(reg) p,
        out(reg) _,
        options(att_syntax, nostack, preserves_flags, pure, readonly)
    );
    p
}

#[cfg(not(windows))]
#[inline(always)]
unsafe fn node_cache() -> *mut NodeCache {
    let p: *mut NodeCache;
    core::arch::asm!(
        "movq %fs:0, {0}",
        "leaq isc_node_thread@tpoff({0}), {0}",
        out(reg) p,
        options(att_syntax, nostack, preserves_flags, pure, readonly)
    );
    p
}

/// A cell for a node of `bytes` bytes (0 < bytes <= NODE_MAX); null when
/// the memory is exhausted.
#[inline(always)]
pub unsafe fn node_alloc(bytes: usize) -> *mut u8 {
    let class = (bytes + 15) / 16 - 1;
    let nc = node_cache();
    let h = (*nc).heads[class];
    if !h.is_null() {
        (*nc).heads[class] = *(h as *mut *mut u8);
        return h;
    }
    node_carve(nc, (class + 1) * 16)
}

#[cold]
#[inline(never)]
unsafe fn node_carve(nc: *mut NodeCache, size: usize) -> *mut u8 {
    if (*nc).bump.is_null() || ((*nc).end as usize) - ((*nc).bump as usize) < size {
        let r = alloc(NODE_REGION, false);
        if r.is_null() {
            return null_mut();
        }
        (*nc).bump = r;
        (*nc).end = r.add(NODE_REGION);
    }
    let p = (*nc).bump;
    (*nc).bump = p.add(size);
    p
}

/// Gives back the cell of a node of `bytes` bytes.
#[inline(always)]
pub unsafe fn node_free(p: *mut u8, bytes: usize) {
    let class = (bytes + 15) / 16 - 1;
    let nc = node_cache();
    *(p as *mut *mut u8) = (*nc).heads[class];
    (*nc).heads[class] = p;
}

#[cfg(windows)]
#[inline(always)]
unsafe fn thread_cache() -> *mut ThreadCache {
    let p: *mut ThreadCache;
    core::arch::asm!(
        "movq %gs:0x58, {0}",
        "movl _tls_index(%rip), {1:e}",
        "movq ({0},{1},8), {0}",
        "leaq isc_heap_thread@SECREL32({0}), {0}",
        out(reg) p,
        out(reg) _,
        options(att_syntax, nostack, preserves_flags, pure, readonly)
    );
    p
}

#[cfg(not(windows))]
#[inline(always)]
unsafe fn thread_cache() -> *mut ThreadCache {
    let p: *mut ThreadCache;
    // the x86-64 TLS ABI: %fs:0 holds the thread pointer itself
    core::arch::asm!(
        "movq %fs:0, {0}",
        "leaq isc_heap_thread@tpoff({0}), {0}",
        out(reg) p,
        options(att_syntax, nostack, preserves_flags, pure, readonly)
    );
    p
}

struct State {
    /// Large classes: the cache (the small classes are in SMALL_FREE).
    free: [*mut Header; CLASSES],
    // the idle list and its bytes
    idle_newest: *mut Header,
    idle_oldest: *mut Header,
    idle: usize,
    // medium blocks: TLSF
    fl_map: u64,
    sl_map: [u8; FL],
    heads: [[*mut Header; 8]; FL],
    medium_in_use: usize,
    // large blocks
    large_newest: *mut Header,
    large_oldest: *mut Header,
    large_cached: usize,
    large_in_use: usize,
    /// Bytes of new memory the program had to get so far: medium blocks
    /// never written, new mappings of large blocks. The clock of the ages of
    /// idle and cached blocks: a block that stayed unused while the program
    /// needed that much new memory is of no use for the sizes it asks for.
    grown: usize,
    colors: usize,
}

static LOCK: AtomicBool = AtomicBool::new(false);
static mut STATE: State = State {
    free: [null_mut(); CLASSES],
    idle_newest: null_mut(),
    idle_oldest: null_mut(),
    idle: 0,
    fl_map: 0,
    sl_map: [0; FL],
    heads: [[null_mut(); 8]; FL],
    medium_in_use: 0,
    large_newest: null_mut(),
    large_oldest: null_mut(),
    large_cached: 0,
    large_in_use: 0,
    grown: 0,
    colors: 0,
};

/// 0: no key yet, 1: being created, 2: created, 3: failed.
static KEY_STATE: AtomicU32 = AtomicU32::new(0);
static mut KEY: c_uint = 0;

fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

unsafe fn state() -> &'static mut State {
    &mut *core::ptr::addr_of_mut!(STATE)
}

#[cold]
fn invalid_free() -> ! {
    super::die(0, 0, &Panic::InvalidFree)
}

unsafe fn map(len: usize, flags: c_int) -> *mut u8 {
    let p = mmap(null_mut(), len, PROT_READ_WRITE, MAP_PRIVATE_ANONYMOUS | flags, -1, 0);
    if p as isize == -1 {
        null_mut()
    } else {
        p as *mut u8
    }
}

unsafe fn at(h: *mut Header, off: usize) -> *mut Header {
    (h as *mut u8).add(off) as *mut Header
}

// ---------------------------------------------------------------------
// the idle list: the free medium blocks with written pages

/// When the free medium block `h` is all of its region: the start of the
/// mapping of the region (kept by its sentinel); null otherwise.
unsafe fn whole_region(h: *mut Header) -> *mut u8 {
    if !(*h).base.is_null() {
        return null_mut();
    }
    let next = at(h, (*h).len);
    if (*next).len == PAGE && (*next).state == IN_USE {
        (*next).next as *mut u8
    } else {
        null_mut()
    }
}

unsafe fn idle_bytes(h: *mut Header) -> usize {
    (*h).dirty as usize * PAGE
}

unsafe fn idle_push(st: &mut State, h: *mut Header) {
    (*h).idle = 1;
    (*h).stamp = (st.grown >> 20) as u32;
    (*h).newer = null_mut();
    (*h).older = st.idle_newest;
    if st.idle_newest.is_null() {
        st.idle_oldest = h;
    } else {
        (*st.idle_newest).newer = h;
    }
    st.idle_newest = h;
    st.idle += idle_bytes(h);
}

unsafe fn idle_remove(st: &mut State, h: *mut Header) {
    if (*h).newer.is_null() {
        st.idle_newest = (*h).older;
    } else {
        (*(*h).newer).older = (*h).older;
    }
    if (*h).older.is_null() {
        st.idle_oldest = (*h).newer;
    } else {
        (*(*h).older).newer = (*h).newer;
    }
    (*h).idle = 0;
    st.idle -= idle_bytes(h);
}

/// Gives back the pages of the blocks idle the longest when there are too
/// many idle bytes, and those of the blocks gone stale: idle while the
/// program had to get more than IDLE_MIN of new memory (`State::grown`).
/// Phases of growing sizes: freed arrays of 10 to 30 MB stayed until the
/// arrays of 40 and 50 MB came, 160 MiB at the peak against 154 MiB with
/// glibc (DECISIONS D81). Lock held.
unsafe fn trim_idle(st: &mut State) {
    let limit = IDLE_MIN + st.medium_in_use + st.large_in_use;
    let target = if st.idle > limit { limit - limit / 4 } else { st.idle };
    let stale = (IDLE_MIN >> 20) as u32;
    let now = (st.grown >> 20) as u32;
    loop {
        let h = st.idle_oldest;
        if h.is_null() || (st.idle <= target && now.wrapping_sub((*h).stamp) <= stale) {
            break;
        }
        idle_remove(st, h);
        // A region that is all free goes back whole: its huge pages that hold
        // the headers of free blocks would stay otherwise (phases of growing
        // sizes: 4096 kB left in a region that no block used any more).
        let region = whole_region(h);
        if !region.is_null() {
            medium_remove(st, h);
            munmap(region as *mut c_void, REGION + HUGE);
            continue;
        }
        // Only whole huge pages go back: giving back a part of one makes the
        // kernel split it, and every later use of the block faults 512 pages
        // of 4 KiB instead of one huge page (DECISIONS D81). The rest of the
        // block, and a block too small to hold a huge page, stay; they are
        // idle again when the block is freed or merged.
        let start = (h as usize + PAGE).next_multiple_of(HUGE);
        let end = (h as usize + (*h).len) & !(HUGE - 1);
        if end > start {
            madvise(start as *mut c_void, end - start, MADV_DONTNEED);
            (*h).contents = RELEASED;
        }
        (*h).dirty = 0;
    }
}

// ---------------------------------------------------------------------
// small blocks

/// Makes the end of the calling thread give its cache back: by a key of
/// the threads of Windows or glibc; the threads of the runtime under Linux
/// give it back themselves (`thread_exit`, D289).
#[cold]
unsafe fn register(tc: *mut ThreadCache) {
    (*tc).registered = 1;
    #[cfg(windows)]
    register_with_key(tc as *mut c_void);
    #[cfg(not(windows))]
    (super::process::os().register)(tc as *mut c_void);
}

/// The end of the calling thread of the runtime (D289): its cache given
/// back to the global lists, if it has one.
#[cfg(not(windows))]
pub fn thread_exit() {
    // SAFETY: the cache of the calling thread, which ends
    unsafe {
        let tc = thread_cache();
        if (*tc).registered != 0 {
            thread_end(tc as *mut c_void);
            (*tc).registered = 0;
        }
    }
}

/// `register` by a key whose destructor gives the cache back.
///
/// # Safety
/// `tc` is the cache of the calling thread.
pub unsafe fn register_with_key(tc: *mut c_void) {
    loop {
        match KEY_STATE.load(Ordering::Acquire) {
            2 => break,
            3 => return,
            0 => {
                if KEY_STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    let ok = pthread_key_create(core::ptr::addr_of_mut!(KEY), Some(thread_end)) == 0;
                    KEY_STATE.store(if ok { 2 } else { 3 }, Ordering::Release);
                }
            }
            _ => core::hint::spin_loop(),
        }
    }
    pthread_setspecific(KEY, tc as *const c_void);
}

/// The global free lists of the small classes, each under its own lock, so
/// that threads that refill or spill different classes do not wait for each
/// other. The chunk being carved is under CHUNK_LOCK, taken last.
static mut SMALL_FREE: [*mut Header; SMALL_CLASSES] = [null_mut(); SMALL_CLASSES];
static CLASS_LOCKS: [AtomicBool; SMALL_CLASSES] = [const { AtomicBool::new(false) }; SMALL_CLASSES];
static CHUNK_LOCK: AtomicBool = AtomicBool::new(false);
/// The chunk being carved of each group of sizes (`group`).
static mut BUMP: [usize; 2] = [0; 2];
static mut BUMP_END: [usize; 2] = [0; 2];

/// Arrays (from ALIGNED_MIN on) and smaller blocks are carved from different
/// chunks: the small blocks a call allocates and its tasks write (the cells
/// of the results of a `reduce`) are never just after an array, whose end
/// the prefetcher of the core that reads it would fetch with them
/// (DECISIONS D81).
fn group(size: usize) -> usize {
    (size >= ALIGNED_MIN) as usize
}

fn spin_lock(l: &AtomicBool) {
    while l.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn lock_small(class: usize) {
    spin_lock(&CLASS_LOCKS[class])
}

fn unlock_small(class: usize) {
    CLASS_LOCKS[class].store(false, Ordering::Release)
}

unsafe fn small_head(class: usize) -> *mut *mut Header {
    core::ptr::addr_of_mut!(SMALL_FREE[class])
}

/// The end of a thread: its cache goes back to the global lists.
unsafe extern "C" fn thread_end(p: *mut c_void) {
    let tc = p as *mut ThreadCache;
    for c in 0..SMALL_CLASSES {
        lock_small(c);
        let mut h = (*tc).heads[c];
        while !h.is_null() {
            let next = (*h).next;
            give_small(h);
            h = next;
        }
        (*tc).heads[c] = null_mut();
        (*tc).counts[c] = 0;
        unlock_small(c);
    }
    for i in 0..2 {
        let h = (*tc).medium[i];
        (*tc).medium[i] = null_mut();
        if !h.is_null() {
            free_medium(h);
        }
    }
}

/// The chunks made (under CHUNK_LOCK).
static mut CHUNKS_MADE: usize = 0;
/// The start of the first chunk, in pages of 4 KiB (`chunk_start`).
static mut FIRST_CHUNK: usize = 0;
/// The small blocks of less than ALIGNED_MIN bytes carved from the spans of
/// the threads (`carve_many`), once the first chunk can hold no more.
static SMALL_SPANS: AtomicBool = AtomicBool::new(false);

/// The start of a new chunk mapped at `base` (with a huge page more, to
/// align it): in huge pages, except the first chunk of the process, in
/// pages of 4 KiB even where the kernel gives huge pages to any mapping
/// (`transparent_hugepage=always`): a small program touches a few pages of
/// it, and the first fault of a huge page clears 2 MiB (D289).
///
/// # Safety
/// Under CHUNK_LOCK; `base` a new mapping of `CHUNK + HUGE` bytes.
unsafe fn chunk_start(base: *mut u8) -> usize {
    let start = (base as usize).next_multiple_of(HUGE);
    // SAFETY: see above
    unsafe {
        if CHUNKS_MADE == 0 {
            FIRST_CHUNK = start;
        }
        let advice = if CHUNKS_MADE == 0 { MADV_NOHUGEPAGE } else { MADV_HUGEPAGE };
        madvise(start as *mut c_void, CHUNK, advice);
        CHUNKS_MADE += 1;
    }
    start
}

/// A new small block, carved from the current chunk.
unsafe fn carve(class: usize, size: usize) -> *mut Header {
    let need = HEADER + size;
    let g = group(size);
    spin_lock(&CHUNK_LOCK);
    if BUMP[g] + need > BUMP_END[g] {
        // the rest of the current chunk is lost (less than 256 KiB)
        let base = map(CHUNK + HUGE, 0);
        if base.is_null() {
            CHUNK_LOCK.store(false, Ordering::Release);
            return null_mut();
        }
        let start = chunk_start(base);
        BUMP[g] = start;
        BUMP_END[g] = start + CHUNK;
    }
    if size >= ALIGNED_MIN {
        // the array starts 64 or 2112 bytes after a page boundary (see
        // PAGE_OFFSETS)
        let off = (BUMP[g] + HEADER) % PAGE;
        let pad = if off <= 64 {
            64 - off
        } else if off <= 2112 {
            2112 - off
        } else {
            PAGE + 64 - off
        };
        if BUMP[g] + pad + need > BUMP_END[g] {
            BUMP_END[g] = BUMP[g];
            CHUNK_LOCK.store(false, Ordering::Release);
            return carve(class, size);
        }
        BUMP[g] += pad;
    }
    let h = BUMP[g] as *mut Header;
    BUMP[g] += need;
    CHUNK_LOCK.store(false, Ordering::Release);
    (*h).class = class as u32;
    (*h).state = FREE;
    (*h).contents = ZERO;
    (*h).idle = 0;
    (*h).next = null_mut();
    h
}

/// Up to `count` new small blocks of class `class` for the thread of `tc`,
/// as a list (by `next`); null if none could be had. Those of ALIGNED_MIN
/// bytes or more keep the placement of `carve`; the others come from the
/// first chunk while it holds them (in pages of 4 KiB, the blocks of all
/// the threads one after the other under one lock), then from the span of
/// the thread, without a lock (`take_span`, D294).
unsafe fn carve_many(tc: *mut ThreadCache, class: usize, size: usize, count: usize) -> *mut Header {
    let mut list: *mut Header = null_mut();
    if size >= ALIGNED_MIN || count == 0 {
        for _ in 0..count {
            let h = carve(class, size);
            if h.is_null() {
                break;
            }
            (*h).next = list;
            list = h;
        }
        return list;
    }
    let need = HEADER + size;
    // in the first chunk, in pages of 4 KiB, the blocks of all the threads
    // carved one after the other under one lock, as before the spans (a
    // page touched by two threads at once is cleared twice at most 4 KiB):
    // the few blocks of a small program stay together
    if !SMALL_SPANS.load(Ordering::Relaxed) {
        spin_lock(&CHUNK_LOCK);
        if BUMP_END[0] == 0 && CHUNKS_MADE == 0 {
            let base = map(CHUNK + HUGE, 0);
            if !base.is_null() {
                let start = chunk_start(base);
                BUMP[0] = start;
                BUMP_END[0] = start + CHUNK;
            }
        }
        let first = FIRST_CHUNK != 0 && BUMP_END[0] == FIRST_CHUNK + CHUNK && BUMP[0] + count * need <= BUMP_END[0];
        if first {
            for _ in 0..count {
                let h = BUMP[0] as *mut Header;
                BUMP[0] += need;
                (*h).class = class as u32;
                (*h).state = FREE;
                (*h).contents = ZERO;
                (*h).idle = 0;
                (*h).next = list;
                list = h;
            }
        } else {
            // the first chunk full (or not the one of these blocks): spans
            // from now on, without this lock
            SMALL_SPANS.store(true, Ordering::Relaxed);
        }
        CHUNK_LOCK.store(false, Ordering::Release);
        if first {
            return list;
        }
    }
    for _ in 0..count {
        if (*tc).span + need > (*tc).span_end {
            let (start, end) = take_span(need);
            if start == 0 {
                break;
            }
            (*tc).span = start;
            (*tc).span_end = end;
        }
        let h = (*tc).span as *mut Header;
        (*tc).span += need;
        (*h).class = class as u32;
        (*h).state = FREE;
        (*h).contents = ZERO;
        (*h).idle = 0;
        (*h).next = list;
        list = h;
    }
    list
}

/// A new span for the small blocks of less than ALIGNED_MIN bytes of one
/// thread, of at least `need` bytes: the chunk from where its last span
/// ended to the end of the huge page that holds `need` bytes more (a new
/// chunk when they do not fit), under the lock of the chunks; (0, 0) if the
/// system has no memory left. The spans end at the ends of huge pages, so
/// that each huge page belongs to one thread: the first write into it,
/// which makes the kernel clear its 2 MiB, is never made by two threads at
/// once, each clearing a page of its own and all but one for nothing
/// (D294: 4 threads writing the same new huge pages took 2.3 times the time
/// and the CPU of each writing its own; carved under the lock, they waited
/// for it). Each span starts at its color (`SPAN_COLORS`).
unsafe fn take_span(need: usize) -> (usize, usize) {
    spin_lock(&CHUNK_LOCK);
    if BUMP[0] + need > BUMP_END[0] {
        // the rest of the current chunk is lost (less than ALIGNED_MIN)
        let base = map(CHUNK + HUGE, 0);
        if base.is_null() {
            CHUNK_LOCK.store(false, Ordering::Release);
            return (0, 0);
        }
        let start = chunk_start(base);
        BUMP[0] = start;
        BUMP_END[0] = start + CHUNK;
    }
    let start = BUMP[0];
    let end = (start + need).next_multiple_of(HUGE).min(BUMP_END[0]);
    BUMP[0] = end;
    // the colors of the spans (D294): each one starts SPAN_COLOR bytes
    // further into its huge page than the one before, modulo SPAN_COLORS of
    // them, when that leaves room for the block
    let color = SPANS % SPAN_COLORS * SPAN_COLOR;
    SPANS += 1;
    CHUNK_LOCK.store(false, Ordering::Release);
    if start + color + need <= end {
        (start + color, end)
    } else {
        (start, end)
    }
}

/// The spans taken (under CHUNK_LOCK), for their colors.
static mut SPANS: usize = 0;
/// The colors of the spans: offsets of 0, SPAN_COLOR, ... into their first
/// huge page, so that the blocks that threads carve in the same order from
/// their spans are not at the same places of their huge pages (D294: at the
/// same places, binary-trees on 4 threads had twice the clears of the
/// pipeline for the order of memory, 3.6 % slower).
const SPAN_COLORS: usize = 8;
const SPAN_COLOR: usize = (8 << 10) + 64 * 3;

/// A free small block goes to the global list of its class. The lock of the
/// class held.
unsafe fn give_small(h: *mut Header) {
    let c = (*h).class as usize;
    (*h).next = *small_head(c);
    *small_head(c) = h;

}

/// The cache of class `class` of the thread is empty: takes up to half of
/// its limit from the global list (or new blocks), returns one of them.
#[cold]
unsafe fn refill(tc: *mut ThreadCache, class: usize, size: usize) -> *mut Header {
    let batch = (THREAD_LIMIT[class] / 2).max(1) as usize;
    let mut first: *mut Header = null_mut();
    let mut got = 0usize;
    let mut keep = |h: *mut Header| {
        if first.is_null() {
            first = h;
        } else {
            (*h).next = (*tc).heads[class];
            (*tc).heads[class] = h;
            (*tc).counts[class] += 1;
        }
    };
    lock_small(class);
    while got < batch {
        let h = *small_head(class);
        if h.is_null() {
            break;
        }
        *small_head(class) = (*h).next;
        keep(h);
        got += 1;
    }
    unlock_small(class);
    // the rest new, carved from the span of the thread (D294; under one lock
    // of the chunks for all of them before, D252, and one lock per block
    // before that)
    if got < batch {
        let mut h = carve_many(tc, class, size, batch - got);
        while !h.is_null() {
            let next = (*h).next;
            keep(h);
            h = next;
        }
    }
    first
}

/// The cache of class `class` of the thread is full: the block and half of
/// the cache go to the global list, with one lock.
#[cold]
unsafe fn spill(tc: *mut ThreadCache, class: usize, h: *mut Header) {
    lock_small(class);
    give_small(h);
    for _ in 0..THREAD_LIMIT[class] / 2 {
        let b = (*tc).heads[class];
        if b.is_null() {
            break;
        }
        (*tc).heads[class] = (*b).next;
        (*tc).counts[class] -= 1;
        give_small(b);
    }
    unlock_small(class);
}

// ---------------------------------------------------------------------
// medium blocks: two-level segregated fit

/// First and second level of a size (a multiple of PAGE).
fn levels(size: usize) -> (usize, usize) {
    let fl = (usize::BITS - 1 - size.leading_zeros()) as usize;
    (fl, (size >> (fl - 3)) & 7)
}

unsafe fn medium_insert(st: &mut State, h: *mut Header) {
    let (fl, sl) = levels((*h).len);
    let head = st.heads[fl][sl];
    (*h).next = head;
    (*h).prev = null_mut();
    if !head.is_null() {
        (*head).prev = h;
    }
    st.heads[fl][sl] = h;
    st.sl_map[fl] |= 1 << sl;
    st.fl_map |= 1 << fl;
    if (*h).dirty > 0 {
        idle_push(st, h);
    }
}

unsafe fn medium_remove(st: &mut State, h: *mut Header) {
    let (fl, sl) = levels((*h).len);
    if (*h).prev.is_null() {
        st.heads[fl][sl] = (*h).next;
        if (*h).next.is_null() {
            st.sl_map[fl] &= !(1 << sl);
            if st.sl_map[fl] == 0 {
                st.fl_map &= !(1 << fl);
            }
        }
    } else {
        (*(*h).prev).next = (*h).next;
    }
    if !(*h).next.is_null() {
        (*(*h).next).prev = (*h).prev;
    }
    if (*h).idle != 0 {
        idle_remove(st, h);
    }
}

/// A free block of at least `size` bytes, or null. Lock held.
unsafe fn medium_find(st: &mut State, size: usize) -> *mut Header {
    // round up to the next second-level size: every block of that list fits
    let (f, _) = levels(size);
    let (mut fl, sl) = levels(size + (1 << (f - 3)) - 1);
    if fl >= FL {
        return null_mut();
    }
    let mut slm = st.sl_map[fl] & (0xFFu8 << sl);
    if slm == 0 {
        let flm = if fl + 1 >= 64 { 0 } else { st.fl_map & (!0u64 << (fl + 1)) };
        if flm == 0 {
            return null_mut();
        }
        fl = flm.trailing_zeros() as usize;
        slm = st.sl_map[fl];
    }
    st.heads[fl][slm.trailing_zeros() as usize]
}

/// A new region: one free block, then a sentinel in use. Lock held.
unsafe fn new_region(st: &mut State) -> bool {
    let base = map(REGION + HUGE, MAP_NORESERVE);
    if base.is_null() {
        return false;
    }
    let start = (base as usize).next_multiple_of(HUGE);
    madvise(start as *mut c_void, REGION, MADV_HUGEPAGE);
    let first = start as *mut Header;
    let sentinel = (start + REGION - PAGE) as *mut Header;
    sentinel.write(Header {
        class: MEDIUM,
        state: IN_USE,
        contents: DIRTY,
        idle: 0,
        _pad: 0,
        // the sentinel keeps the start of the mapping (see `whole_region`)
        next: base as *mut Header,
        prev: null_mut(),
        newer: null_mut(),
        older: null_mut(),
        base: first,
        len: PAGE,
        dirty: 0,
        stamp: 0,
    });
    first.write(Header {
        class: MEDIUM,
        state: FREE,
        contents: ZERO,
        idle: 0,
        _pad: 0,
        next: null_mut(),
        prev: null_mut(),
        newer: null_mut(),
        older: null_mut(),
        base: null_mut(),
        len: REGION - PAGE,
        dirty: 0,
        stamp: 0,
    });
    medium_insert(st, first);
    true
}

/// The contents of two free blocks merged into one.
fn merged(a: u8, b: u8) -> u8 {
    if a == DIRTY || b == DIRTY {
        DIRTY
    } else if a == RELEASED || b == RELEASED {
        RELEASED
    } else {
        ZERO
    }
}

#[cold]
unsafe fn alloc_medium(bytes: usize) -> *mut Header {
    let need = (HEADER + bytes).next_multiple_of(PAGE);
    lock();
    let st = state();
    let mut h = medium_find(st, need);
    if h.is_null() {
        if !new_region(st) {
            unlock();
            return null_mut();
        }
        h = medium_find(st, need);
    }
    medium_remove(st, h);
    if (*h).len - need >= SPLIT_MIN {
        // the rest stays free; its header is written where the block was
        // never given out, so a ZERO block stays zero where it is used
        let rest = at(h, need);
        rest.write(Header {
            class: MEDIUM,
            state: FREE,
            contents: (*h).contents,
            idle: 0,
            _pad: 0,
            next: null_mut(),
            prev: null_mut(),
            newer: null_mut(),
            older: null_mut(),
            base: h,
            len: (*h).len - need,
            // the written bytes are taken to be at the start of the block
            dirty: (*h).dirty.saturating_sub((need / PAGE) as u32),
            stamp: 0,
        });
        (*at(rest, (*rest).len)).base = rest;
        (*h).len = need;
        medium_insert(st, rest);
    }
    st.medium_in_use += (*h).len;
    if (*h).contents == ZERO {
        st.grown += (*h).len;
    }
    trim_idle(st);
    // under the lock: the neighbours read the state of a medium block when
    // they are freed
    (*h).state = IN_USE;
    unlock();
    h
}

#[cold]
unsafe fn free_medium(mut h: *mut Header) {
    lock();
    let st = state();
    (*h).state = FREE;
    st.medium_in_use -= (*h).len;
    (*h).contents = DIRTY;
    (*h).dirty = ((*h).len / PAGE) as u32;
    let next = at(h, (*h).len);
    if (*next).state == FREE {
        medium_remove(st, next);
        (*h).contents = merged((*h).contents, (*next).contents);
        (*h).len += (*next).len;
        (*h).dirty += (*next).dirty;
        // the header of `next` is now inside the block
        core::ptr::write_bytes(next as *mut u8, 0, HEADER);
    }
    let prev = (*h).base;
    if !prev.is_null() && (*prev).state == FREE {
        medium_remove(st, prev);
        (*prev).contents = merged((*prev).contents, (*h).contents);
        (*prev).len += (*h).len;
        (*prev).dirty += (*h).dirty;
        core::ptr::write_bytes(h as *mut u8, 0, HEADER);
        h = prev;
    }
    (*at(h, (*h).len)).base = h;
    if (*h).dirty == 0 && !whole_region(h).is_null() {
        // all of the region is free: idle, so that it goes back once stale
        (*h).dirty = 1;
    }
    medium_insert(st, h);
    trim_idle(st);
    unlock();
}

// ---------------------------------------------------------------------
// large blocks

/// A new large block in its own mapping, null if the system has no memory
/// left. Lock held.
unsafe fn new_large(st: &mut State, class: usize, size: usize) -> *mut Header {
    // The array is in huge pages and starts `color` + 64 bytes after the
    // start of its first one (`color` includes LARGE_OFFSET), its header
    // just before. The color changes at
    // each mapping, by steps of 256 KiB and half a page: two arrays read
    // together (`dot`) must be at least 64 KiB apart modulo 2 MiB, and 0 or
    // 2048 bytes apart modulo 4096 (PAGE_OFFSETS). Measured on the large
    // `dot`, one thread: 8013 µs when they are at the same offset modulo
    // 2 MiB, 7838 µs 4160 bytes apart, 6517 µs 262208 bytes apart, 6887 µs
    // with glibc (DECISIONS D79).
    let color = LARGE_OFFSET + st.colors % COLORS * COLOR_STEP;
    st.colors += 1;
    let len = HEADER + color + size + HUGE;
    let base = map(len, 0);
    if base.is_null() {
        return null_mut();
    }
    let start = (base as usize).next_multiple_of(HUGE);
    let h = (start + color) as *mut Header;
    h.write(Header {
        class: class as u32,
        state: FREE,
        contents: ZERO,
        idle: 0,
        _pad: 0,
        next: null_mut(),
        prev: null_mut(),
        newer: null_mut(),
        older: null_mut(),
        base: base as *mut Header,
        len,
        dirty: 0,
        stamp: 0,
    });
    h
}

/// Huge pages for the huge pages the array of `bytes` bytes covers whole, pages
/// of 4 KiB for the first one when the color leaves a part of it unused, and
/// for the last one, partly used: the mapping covers the size of the class,
/// so that the kernel would otherwise give 2 MiB for each of them (two arrays
/// of 80 MB: 163840 kB resident against 156264 kB with glibc; DECISIONS D81).
/// `dirty` keeps the end of the huge pages, in pages from the start of the
/// first huge page: a cached block taken for a longer array gets more of
/// them.
unsafe fn shape_large(h: *mut Header, bytes: usize) {
    let first = (h as usize) & !(HUGE - 1);
    let user = h as usize + HEADER;
    let end = (user + bytes) & !(HUGE - 1);
    let map_end = (*h).base as usize + (*h).len;
    if (*h).dirty == 0 {
        // the first huge page too when less than a page of it is unused
        // (the color 0: LARGE_OFFSET)
        let from = if h as usize - first < PAGE { first } else { first + HUGE };
        if from > first {
            madvise(first as *mut c_void, HUGE, MADV_NOHUGEPAGE);
        }
        if end > from {
            madvise(from as *mut c_void, end - from, MADV_HUGEPAGE);
        }
        let tail = end.max(from);
        if map_end > tail {
            madvise(tail as *mut c_void, map_end - tail, MADV_NOHUGEPAGE);
        }
        (*h).dirty = ((end.max(from) - first) / PAGE) as u32;
    } else {
        let old = first + (*h).dirty as usize * PAGE;
        if end > old {
            madvise(old as *mut c_void, end - old, MADV_HUGEPAGE);
            (*h).dirty = ((end - first) / PAGE) as u32;
        }
    }
}

/// Takes the large block `h` out of the cache.
unsafe fn take_cached(st: &mut State, h: *mut Header) {
    let c = (*h).class as usize;
    if (*h).prev.is_null() {
        st.free[c] = (*h).next;
    } else {
        (*(*h).prev).next = (*h).next;
    }
    if !(*h).next.is_null() {
        (*(*h).next).prev = (*h).prev;
    }
    if (*h).newer.is_null() {
        st.large_newest = (*h).older;
    } else {
        (*(*h).newer).older = (*h).older;
    }
    if (*h).older.is_null() {
        st.large_oldest = (*h).newer;
    } else {
        (*(*h).older).newer = (*h).newer;
    }
    st.large_cached -= alloc_class_size(c);
}

#[cold]
unsafe fn alloc_large(class: usize, size: usize, bytes: usize) -> *mut Header {
    lock();
    let st = state();
    // a cached block of this class or of one of the 3 next ones (at most
    // 47 % larger)
    let mut h = null_mut();
    for c in class..(class + 4).min(CLASSES) {
        if !st.free[c].is_null() {
            h = st.free[c];
            take_cached(st, h);
            break;
        }
    }
    if h.is_null() {
        // The largest cached block smaller than the request grows to it
        // (D211): its pages, written, stay; only the rest is new memory
        // that the system clears. Arrays that grow from one phase to the
        // next (reverse-complement: 50, 74, then 123 MB) then reuse the
        // pages of the phase before.
        // The other cached blocks smaller than the request stay: the next
        // request of the phase grows the largest of them in turn (two
        // arrays per phase, D220); `trim_large` bounds what the cache holds.
        h = grow_cached(st, class, size);
        if h.is_null() {
            h = new_large(st, class, size);
            st.grown += size;
        }
    }
    if !h.is_null() {
        st.large_in_use += alloc_class_size((*h).class as usize);
        shape_large(h, bytes);
    }
    trim_large(st);
    trim_idle(st);
    unlock();
    h
}

/// The largest cached large block smaller than `size`, out of the cache,
/// its mapping grown (`mremap`) so that it holds a block of class `class`;
/// null if there is none or the system refuses. The mapping moves to an
/// address of the same offset modulo HUGE, so that its huge pages move
/// whole (their tables are moved, not split). Its contents stay (DIRTY).
/// Lock held.
unsafe fn grow_cached(st: &mut State, class: usize, size: usize) -> *mut Header {
    const MREMAP_MAYMOVE: c_int = 1;
    const MREMAP_FIXED: c_int = 2;
    let mut best: *mut Header = null_mut();
    let mut b = st.large_oldest;
    while !b.is_null() {
        if alloc_class_size((*b).class as usize) < size && (best.is_null() || (*b).len > (*best).len) {
            best = b;
        }
        b = (*b).newer;
    }
    if best.is_null() {
        return null_mut();
    }
    take_cached(st, best);
    let base = (*best).base as usize;
    let old_len = (*best).len;
    let off = best as usize - base;
    let new_len = (off + HEADER + size + HUGE).next_multiple_of(PAGE);
    // a place of new_len bytes at the same offset modulo HUGE
    let res = map(new_len + HUGE, 0);
    if res.is_null() {
        munmap(base as *mut c_void, old_len);
        return null_mut();
    }
    let dst = (res as usize).next_multiple_of(HUGE) + base % HUGE;
    let dst = if dst + new_len > res as usize + new_len + HUGE { dst - HUGE } else { dst };
    // one advice on the whole mapping: the kernel merges the areas that
    // `shape_large` made (`mremap` moves one area only, EFAULT otherwise)
    madvise(base as *mut c_void, old_len, MADV_HUGEPAGE);
    let moved = mremap(base as *mut c_void, old_len, new_len, MREMAP_MAYMOVE | MREMAP_FIXED, dst as *mut c_void);
    if moved as isize == -1 {
        munmap(res as *mut c_void, new_len + HUGE);
        munmap(base as *mut c_void, old_len);
        return null_mut();
    }
    // the rest of the place
    if dst > res as usize {
        munmap(res as *mut c_void, dst - res as usize);
    }
    let end = res as usize + new_len + HUGE;
    if end > dst + new_len {
        munmap((dst + new_len) as *mut c_void, end - (dst + new_len));
    }
    let h = (dst + off) as *mut Header;
    (*h).class = class as u32;
    (*h).state = FREE;
    (*h).contents = DIRTY;
    (*h).base = dst as *mut Header;
    (*h).len = new_len;
    // `shape_large` advises its parts again
    (*h).dirty = 0;
    st.grown += new_len - old_len;
    h
}

/// Unmaps the cached large blocks freed the longest ago while the cache holds
/// more than max(CACHE_MIN, the bytes of the large blocks in use), and those
/// gone stale: cached while the program had to get more than CACHE_MIN plus
/// the large bytes in use of new memory (`State::grown`). A kernel takes its temporary again at
/// its next call; a block of a size no longer asked for goes back to the
/// system. Lock held.
unsafe fn trim_large(st: &mut State) {
    let limit = st.large_in_use.max(CACHE_MIN);
    let stale = ((CACHE_MIN + st.large_in_use) >> 20) as u32;
    let now = (st.grown >> 20) as u32;
    loop {
        let old = st.large_oldest;
        if old.is_null() || (st.large_cached <= limit && now.wrapping_sub((*old).stamp) <= stale) {
            break;
        }
        take_cached(st, old);
        munmap((*old).base as *mut c_void, (*old).len);
    }
}

/// `bytes` of memory that the program holds outside the allocator (the
/// standard input mapped from its file, D257) counted with the large blocks
/// in use (`held`), or no longer: the cache of large blocks and the idle
/// bytes are bounded by them, and keep what they would have kept had the
/// allocator given that memory (regex-redux, its input of 50.8 MB mapped
/// and not counted: 1820 -> 3696 page faults, 294.1 -> 313.1 ms).
pub fn held_outside(bytes: usize, held: bool) {
    lock();
    let st = unsafe { state() };
    if held {
        st.large_in_use += bytes;
    } else {
        st.large_in_use -= bytes;
    }
    unlock();
}

#[cold]
unsafe fn free_large(h: *mut Header) {
    let class = (*h).class as usize;
    let size = alloc_class_size(class);
    lock();
    let st = state();
    st.large_in_use -= size;
    (*h).next = st.free[class];
    (*h).prev = null_mut();
    if !(*h).next.is_null() {
        (*(*h).next).prev = h;
    }
    st.free[class] = h;
    (*h).older = st.large_newest;
    (*h).newer = null_mut();
    if st.large_newest.is_null() {
        st.large_oldest = h;
    } else {
        (*st.large_newest).newer = h;
    }
    st.large_newest = h;
    st.large_cached += size;
    (*h).stamp = (st.grown >> 20) as u32;
    trim_large(st);
    trim_idle(st);
    unlock();
}

// ---------------------------------------------------------------------
// entry points

/// `bytes` bytes (>= 1) aligned on 64 bytes, zeroed if `zero`; null if the
/// size is too large or the system has no memory left.
#[inline]
/// The common case (D165): a block of `bytes` (a multiple of 64, at most
/// 1 KiB), not cleared, found in the cache of the thread; null otherwise
/// (the caller then takes the complete path, which also checks the block).
#[inline(always)]
pub unsafe fn alloc_cached(bytes: usize) -> *mut u8 {
    let class = bytes / 64 - 1;
    let tc = thread_cache();
    let h = (*tc).heads[class];
    if !h.is_null() && (*tc).registered != 0 && (*h).state == FREE {
        (*tc).heads[class] = (*h).next;
        (*tc).counts[class] -= 1;
        (*h).state = IN_USE;
        (*h).contents = DIRTY;
        return (h as *mut u8).add(HEADER);
    }
    null_mut()
}

/// The common case of `free` (D165): a small block in use, its class not
/// full in the cache of the thread; false otherwise.
#[inline(always)]
pub unsafe fn free_cached(p: *mut u8) -> bool {
    if !p.is_null() && p as usize % 64 == 0 {
        let h = p.sub(HEADER) as *mut Header;
        let class = (*h).class as usize;
        if class < SMALL_CLASSES && (*h).state == IN_USE {
            let tc = thread_cache();
            if (*tc).registered != 0 && (*tc).counts[class] < THREAD_LIMIT[class] {
                (*h).state = FREE;
                (*h).next = (*tc).heads[class];
                (*tc).heads[class] = h;
                (*tc).counts[class] += 1;
                return true;
            }
        }
    }
    false
}

#[cold]
#[inline(never)]
pub unsafe fn alloc(bytes: usize, zero: bool) -> *mut u8 {
    let h = if bytes <= SMALL_MAX {
        let Some((class, size)) = alloc_class(bytes) else {
            return null_mut();
        };
        let tc = thread_cache();
        if (*tc).registered == 0 {
            register(tc);
        }
        let h = (*tc).heads[class];
        if h.is_null() {
            refill(tc, class, size)
        } else {
            (*tc).heads[class] = (*h).next;
            (*tc).counts[class] -= 1;
            h
        }
    } else if bytes <= MEDIUM_MAX {
        let need = (HEADER + bytes).next_multiple_of(PAGE);
        let tc = thread_cache();
        let mut h = null_mut();
        for i in [1, 0] {
            let b = (*tc).medium[i];
            if !b.is_null() && (*b).len >= need && (*b).len - need <= need / 4 {
                (*tc).medium[i] = null_mut();
                if (*b).state != CACHED {
                    invalid_free();
                }
                (*b).state = IN_USE;
                h = b;
                break;
            }
        }
        if h.is_null() {
            alloc_medium(bytes)
        } else {
            h
        }
    } else {
        let Some((class, size)) = alloc_class(bytes) else {
            return null_mut();
        };
        alloc_large(class, size, bytes)
    };
    if h.is_null() {
        return null_mut();
    }
    if (*h).class != MEDIUM {
        // (a medium block is marked under the lock, by alloc_medium)
        if (*h).state != FREE {
            invalid_free();
        }
        (*h).state = IN_USE;
    }
    let user = (h as *mut u8).add(HEADER);
    if zero {
        if (*h).contents == ZERO {
            // Pages never written are zero, but they are not there yet: one
            // write per page makes the kernel give them for writing, in huge
            // pages where it can. A program that reads a `zeros` array first
            // would otherwise get the shared zero page, and each of its later
            // writes would fault one page of 4 KiB (a block of 40 MiB read
            // then written: 9729 faults and 2048 kB of huge pages, against
            // none and 38912 kB with glibc, which clears it; DECISIONS D81).
            let mut a = user as usize;
            let end = a + bytes;
            while a < end {
                core::ptr::write_volatile(a as *mut u8, 0);
                a = (a + 1).next_multiple_of(PAGE);
            }
        } else {
            core::ptr::write_bytes(user, 0, bytes);
        }
    }
    (*h).contents = DIRTY;
    user
}

#[cold]
#[inline(never)]
pub unsafe fn free(p: *mut u8) {
    if p.is_null() {
        return;
    }
    if p as usize % 64 != 0 {
        invalid_free();
    }
    let h = p.sub(HEADER) as *mut Header;
    let class = (*h).class;
    if (*h).state != IN_USE || (class as usize >= CLASSES && class != MEDIUM) {
        invalid_free();
    }
    if class != MEDIUM {
        // (a medium block is marked under the lock, by free_medium)
        (*h).state = FREE;
    }
    if (class as usize) < SMALL_CLASSES {
        let class = class as usize;
        let tc = thread_cache();
        if (*tc).registered == 0 {
            register(tc);
        }
        if (*tc).counts[class] < THREAD_LIMIT[class] {
            (*h).next = (*tc).heads[class];
            (*tc).heads[class] = h;
            (*tc).counts[class] += 1;
        } else {
            spill(tc, class, h);
        }
    } else if class == MEDIUM {
        if (*h).len <= MEDIUM_CACHED {
            // kept by the thread, the older cached block goes back
            let tc = thread_cache();
            if (*tc).registered == 0 {
                register(tc);
            }
            (*h).state = CACHED;
            if (*tc).medium[1].is_null() {
                (*tc).medium[1] = h;
            } else {
                let old = (*tc).medium[0];
                (*tc).medium[0] = (*tc).medium[1];
                (*tc).medium[1] = h;
                if !old.is_null() {
                    free_medium(old);
                }
            }
        } else {
            free_medium(h);
        }
    } else {
        free_large(h);
    }
}
