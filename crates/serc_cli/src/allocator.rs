//! Route ordinary Rust layouts through mimalloc's default-alignment entry points.

#![allow(
    unsafe_code,
    reason = "implementing the GlobalAlloc contract requires unsafe FFI"
)]

use std::alloc::{GlobalAlloc, Layout};

/// The CLI allocator, backed by mimalloc for both ordinary and exceptional alignments.
pub(super) struct MiMalloc;

#[inline]
fn has_default_alignment(size: usize, alignment: usize) -> bool {
    // The C allocator can satisfy native-word alignment if the allocation is large enough
    // to contain a word or an equivalently aligned smaller type. Larger or under-sized
    // requests use mimalloc's alignment-aware Rust allocator.
    alignment <= align_of::<usize>() && size >= alignment
}

// SAFETY: All allocation paths are backed by the same mimalloc instance. The default C
// entry points are used only when their size-derived alignment is sufficient; the Rust
// mimalloc allocator handles the remaining layouts. Both paths can be freed by mimalloc.
unsafe impl GlobalAlloc for MiMalloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if has_default_alignment(layout.size(), layout.align()) {
            // SAFETY: The layout is nonempty and the C allocator supplies its alignment.
            unsafe { libmimalloc_sys::mi_malloc(layout.size()).cast() }
        } else {
            // SAFETY: The caller supplied a valid, nonempty layout to this allocator.
            unsafe { mimalloc::MiMalloc.alloc(layout) }
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if has_default_alignment(layout.size(), layout.align()) {
            // SAFETY: The C allocator supplies the layout alignment and zeroes every byte.
            unsafe { libmimalloc_sys::mi_zalloc(layout.size()).cast() }
        } else {
            // SAFETY: The caller supplied a valid, nonempty layout to this allocator.
            unsafe { mimalloc::MiMalloc.alloc_zeroed(layout) }
        }
    }

    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: Every allocation from this allocator belongs to mimalloc, and the caller
        // supplies the live pointer and layout from that allocation.
        unsafe { mimalloc::MiMalloc.dealloc(pointer, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if has_default_alignment(new_size, layout.align()) && layout.size() >= layout.align() {
            // SAFETY: The old allocation belongs to mimalloc and its layout is compatible
            // with the default path; C realloc preserves its contents and supplies the
            // alignment for the new size.
            unsafe { libmimalloc_sys::mi_realloc(pointer.cast(), new_size).cast() }
        } else {
            // SAFETY: Both original allocation paths use mimalloc. Its aligned realloc
            // accepts both kinds of live pointers and preserves the requested alignment.
            unsafe { mimalloc::MiMalloc.realloc(pointer, layout, new_size) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GlobalAlloc, Layout, MiMalloc, has_default_alignment};

    fn assert_aligned(pointer: *mut u8, alignment: usize) {
        assert!(!pointer.is_null());
        assert_eq!(pointer.align_offset(alignment), 0);
    }

    fn test_sizes(alignment: usize) -> [usize; 7] {
        [
            1,
            alignment.saturating_sub(1).max(1),
            alignment,
            alignment + 1,
            31,
            128,
            8193,
        ]
    }

    fn test_alignments() -> impl Iterator<Item = usize> {
        (0..=12).map(|shift| 1 << shift)
    }

    #[test]
    fn default_entry_points_require_both_size_and_native_word_alignment() {
        let word = align_of::<usize>();
        assert!(!has_default_alignment(0, 1));
        assert!(has_default_alignment(1, 1));
        assert!(!has_default_alignment(word - 1, word));
        assert!(has_default_alignment(word, word));
        assert!(has_default_alignment(word + 1, word));
        assert!(!has_default_alignment(word * 4, word * 2));
    }

    #[test]
    fn allocations_and_zeroed_allocations_satisfy_their_layouts() {
        for alignment in test_alignments() {
            for size in test_sizes(alignment) {
                let layout = Layout::from_size_align(size, alignment).unwrap();
                for zeroed in [false, true] {
                    // SAFETY: The layout is valid and nonempty. Only initialized bytes
                    // are read, and the original pointer is freed with its exact layout.
                    unsafe {
                        let pointer = if zeroed {
                            MiMalloc.alloc_zeroed(layout)
                        } else {
                            MiMalloc.alloc(layout)
                        };
                        assert_aligned(pointer, alignment);
                        if zeroed {
                            assert!(
                                std::slice::from_raw_parts(pointer, size)
                                    .iter()
                                    .all(|&byte| byte == 0)
                            );
                        }
                        pointer.write_bytes(0xa5, size);
                        assert!(
                            std::slice::from_raw_parts(pointer, size)
                                .iter()
                                .all(|&byte| byte == 0xa5)
                        );
                        MiMalloc.dealloc(pointer, layout);
                    }
                }
            }
        }
    }

    #[test]
    fn growing_and_shrinking_across_alignment_boundaries_preserves_data() {
        for alignment in test_alignments() {
            let sizes = test_sizes(alignment);
            for size in sizes {
                for new_size in sizes {
                    let layout = Layout::from_size_align(size, alignment).unwrap();
                    let new_layout = Layout::from_size_align(new_size, alignment).unwrap();
                    // SAFETY: Both layouts are valid and nonempty. The original allocation
                    // is initialized, only the preserved bytes are read after reallocating,
                    // and the reallocated pointer is freed with its new layout.
                    unsafe {
                        let pointer = MiMalloc.alloc(layout);
                        assert_aligned(pointer, alignment);
                        for (byte, pattern) in std::slice::from_raw_parts_mut(pointer, size)
                            .iter_mut()
                            .zip((0..=u8::MAX).cycle())
                        {
                            *byte = pattern;
                        }
                        let resized = MiMalloc.realloc(pointer, layout, new_size);
                        assert_aligned(resized, alignment);
                        assert!(
                            std::slice::from_raw_parts(resized, size.min(new_size))
                                .iter()
                                .copied()
                                .eq((0..=u8::MAX).cycle().take(size.min(new_size)))
                        );
                        MiMalloc.dealloc(resized, new_layout);
                    }
                }
            }
        }
    }

    #[test]
    fn repeated_reallocations_can_alternate_between_entry_points() {
        let word = align_of::<usize>();
        for alignment in [2, word, word * 2] {
            let mut layout = Layout::from_size_align(1, alignment).unwrap();
            // SAFETY: Every layout is valid and nonempty. The retained bytes are initialized
            // before being read, and the final live pointer is freed with its current layout.
            unsafe {
                let mut pointer = MiMalloc.alloc(layout);
                assert_aligned(pointer, alignment);
                pointer.write(0xa5);
                for size in [word * 2, 1, word, word * 3, 2] {
                    let next = Layout::from_size_align(size, alignment).unwrap();
                    pointer = MiMalloc.realloc(pointer, layout, size);
                    assert_aligned(pointer, alignment);
                    assert!(
                        std::slice::from_raw_parts(pointer, layout.size().min(size))
                            .iter()
                            .all(|&byte| byte == 0xa5)
                    );
                    pointer.write_bytes(0xa5, size);
                    layout = next;
                }
                MiMalloc.dealloc(pointer, layout);
            }
        }
    }
}
