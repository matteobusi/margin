// Copyright 2026 RISC Zero, Inc.
// Modified for Margin: the TLSF heap is replaced by a size-class allocator.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A reclaiming allocator that costs a few instructions per call.
//!
//! Every request is rounded up to a size class: multiples of 8 bytes up to 512, then
//! powers of two. Each class has a free list threaded through the freed blocks. A block
//! comes from its class's free list if one is available, otherwise from a bump pointer.
//! Freed blocks are reused only for the same class, so memory can fragment across
//! classes, but there is no search, no splitting and no coalescing: in the zkVM every
//! instruction is a cycle, and TLSF's bookkeeping cost about a sixth of our run time.

use crate::{memory::GUEST_MAX_MEM, syscall::sys_panic};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use critical_section::RawRestoreState;

#[global_allocator]
pub static HEAP: SizeClassAlloc = SizeClassAlloc;

struct CriticalSection;
critical_section::set_impl!(CriticalSection);

unsafe impl critical_section::Impl for CriticalSection {
    unsafe fn acquire() -> RawRestoreState {
        // this is a no-op. we're in a single-threaded, non-preemptive context
    }

    unsafe fn release(_token: RawRestoreState) {
        // this is a no-op. we're in a single-threaded, non-preemptive context
    }
}

const SMALL_STEP: usize = 8;
const SMALL_MAX: usize = 512;
const NUM_SMALL: usize = SMALL_MAX / SMALL_STEP;
/// Large classes are powers of two from 2^10 up to 2^31.
const LARGE_MIN_LOG: u32 = 10;
const NUM_LARGE: usize = 22;
/// Alignment of large blocks taken from the bump pointer (and so of every large block).
const LARGE_ALIGN: usize = 4096;

// SAFETY (all statics): the guest is single-threaded and non-preemptive.
static mut POS: usize = 0;
static mut END: usize = 0;
static mut SMALL: [*mut u8; NUM_SMALL] = [null_mut(); NUM_SMALL];
static mut LARGE: [*mut u8; NUM_LARGE] = [null_mut(); NUM_LARGE];

#[derive(Clone, Copy)]
enum Class {
    /// Index into `SMALL`; block size `(i + 1) * 8`, 8-byte aligned.
    Small(usize),
    /// Index into `LARGE`; block size `2^(LARGE_MIN_LOG + i)`, `LARGE_ALIGN`-aligned.
    Large(usize),
}

impl Class {
    #[inline(always)]
    fn of(layout: Layout) -> Class {
        let size = layout.size().max(1);
        if layout.align() <= SMALL_STEP && size <= SMALL_MAX {
            return Class::Small((size + SMALL_STEP - 1) / SMALL_STEP - 1);
        }
        let need = size.max(layout.align()).max(1 << LARGE_MIN_LOG);
        let log = usize::BITS - (need - 1).leading_zeros();
        let i = (log - LARGE_MIN_LOG) as usize;
        if i >= NUM_LARGE || layout.align() > LARGE_ALIGN {
            oom();
        }
        Class::Large(i)
    }

    #[inline(always)]
    fn block(self) -> (usize, usize) {
        match self {
            Class::Small(i) => ((i + 1) * SMALL_STEP, SMALL_STEP),
            Class::Large(i) => (1 << (LARGE_MIN_LOG as usize + i), LARGE_ALIGN),
        }
    }

    #[inline(always)]
    unsafe fn list(self) -> *mut *mut u8 {
        match self {
            Class::Small(i) => unsafe { core::ptr::addr_of_mut!(SMALL[i]) },
            Class::Large(i) => unsafe { core::ptr::addr_of_mut!(LARGE[i]) },
        }
    }
}

fn oom() -> ! {
    const MSG: &[u8] = "Out of memory (size-class allocator)".as_bytes();
    unsafe { sys_panic(MSG.as_ptr(), MSG.len()) };
    #[allow(clippy::empty_loop)]
    loop {}
}

#[inline(always)]
unsafe fn bump(size: usize, align: usize) -> *mut u8 {
    let pos = unsafe { (POS + align - 1) & !(align - 1) };
    match pos.checked_add(size) {
        Some(next) if next <= unsafe { END } => {
            unsafe { POS = next };
            pos as *mut u8
        }
        _ => oom(),
    }
}

/// Take a block of `class`; the flag says whether it is fresh (still zero).
#[inline(always)]
unsafe fn take(class: Class) -> (*mut u8, bool) {
    unsafe {
        let list = class.list();
        let head = *list;
        if !head.is_null() {
            *list = *(head as *mut *mut u8);
            return (head, false);
        }
        let (size, align) = class.block();
        (bump(size, align), true)
    }
}

pub struct SizeClassAlloc;

unsafe impl GlobalAlloc for SizeClassAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { take(Class::of(layout)).0 }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe {
            let (p, fresh) = take(Class::of(layout));
            // zkVM memory starts zeroed and bump memory was never handed out before.
            if !fresh {
                core::ptr::write_bytes(p, 0, layout.size());
            }
            p
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            let list = Class::of(layout).list();
            *(ptr as *mut *mut u8) = *list;
            *list = ptr;
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
            let (old, new) = (Class::of(layout), Class::of(new_layout));
            if old.block() == new.block() {
                return ptr;
            }
            let (p, _) = take(new);
            core::ptr::copy_nonoverlapping(ptr, p, layout.size().min(new_size));
            self.dealloc(ptr, layout);
            p
        }
    }
}

/// Initialize the allocator with the memory above the program image.
///
/// # Safety
///
/// This function must be called exactly once.
pub unsafe fn init() {
    extern "C" {
        static _end: u8;
    }
    unsafe {
        POS = (&_end) as *const u8 as usize;
        END = GUEST_MAX_MEM;
    }
}
