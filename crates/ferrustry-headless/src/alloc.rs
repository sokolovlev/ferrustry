//! Счётчик аллокаций для бенчмарков.
//!
//! Это единственное место в проекте с `unsafe`: реализация `GlobalAlloc` обязана быть
//! `unsafe impl`. Обёртка только считает вызовы и делегирует системному аллокатору.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

/// Аллокатор, считающий выделения памяти.
pub struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: все методы делегируют `System` с теми же аргументами и выполняют те же контракты.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: контракт `alloc` передаётся вызывающему без изменений.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` выделен этим же аллокатором (через `System`) с тем же `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: как в `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `ptr`/`layout` получены от этого аллокатора, контракт передаётся без изменений.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Число выделений с начала процесса.
pub fn count() -> u64 {
    ALLOCS.load(Ordering::Relaxed)
}
