//! Shared capped allocation for portable Brotli decoding.
use std::{
    io::{self, Read},
    rc::Rc,
};
pub(crate) const BROTLI_WORKSPACE: u64 = 64 * 1024 * 1024;
pub(crate) struct BudgetAllocator<T: Clone> {
    heap: brotli::HeapAlloc<T>,
    used: Rc<std::cell::Cell<usize>>,
    failed: Rc<std::cell::Cell<bool>>,
}
#[cfg(feature = "brotli")]
impl<T: Clone + Default> BudgetAllocator<T> {
    fn new(used: &Rc<std::cell::Cell<usize>>, failed: &Rc<std::cell::Cell<bool>>) -> Self {
        Self {
            heap: brotli::HeapAlloc::default(),
            used: used.clone(),
            failed: failed.clone(),
        }
    }
}
#[cfg(feature = "brotli")]
impl<T: Clone> brotli::Allocator<T> for BudgetAllocator<T> {
    type AllocatedMemory = <brotli::HeapAlloc<T> as brotli::Allocator<T>>::AllocatedMemory;
    fn alloc_cell(&mut self, count: usize) -> Self::AllocatedMemory {
        let bytes = count.checked_mul(std::mem::size_of::<T>());
        let total = bytes.and_then(|bytes| self.used.get().checked_add(bytes));
        if total.is_none_or(|total| total as u64 > BROTLI_WORKSPACE) {
            self.failed.set(true);
            return Self::AllocatedMemory::default();
        }
        self.used.set(total.unwrap_or(0));
        self.heap.alloc_cell(count)
    }
    fn free_cell(&mut self, memory: Self::AllocatedMemory) {
        use brotli::SliceWrapper;
        self.used.set(
            self.used
                .get()
                .saturating_sub(std::mem::size_of_val(memory.slice())),
        );
        self.heap.free_cell(memory);
    }
}

#[cfg(feature = "brotli")]
pub(crate) type BoundedBrotli<R> = brotli::DecompressorCustomIo<
    io::Error,
    brotli::IntoIoReader<R>,
    <brotli::HeapAlloc<u8> as brotli::Allocator<u8>>::AllocatedMemory,
    BudgetAllocator<u8>,
    BudgetAllocator<u32>,
    BudgetAllocator<brotli::HuffmanCode>,
>;
#[cfg(feature = "brotli")]
pub(crate) fn brotli_decoder<R: Read>(input: R) -> (BoundedBrotli<R>, Rc<std::cell::Cell<bool>>) {
    use brotli::Allocator;
    let used = Rc::new(std::cell::Cell::new(8192));
    let failed = Rc::new(std::cell::Cell::new(false));
    let buffer = brotli::HeapAlloc::new(0u8).alloc_cell(8192);
    let decoder = brotli::DecompressorCustomIo::new(
        brotli::IntoIoReader(input),
        buffer,
        BudgetAllocator::new(&used, &failed),
        BudgetAllocator::new(&used, &failed),
        BudgetAllocator::new(&used, &failed),
        io::Error::new(io::ErrorKind::InvalidData, "invalid Brotli stream"),
    );
    (decoder, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brotli::{Allocator, SliceWrapper};
    #[test]
    fn allocator_budget_is_shared_across_element_types_and_released() {
        let used = Rc::new(std::cell::Cell::new(BROTLI_WORKSPACE as usize - 16));
        let failed = Rc::new(std::cell::Cell::new(false));
        let mut bytes = BudgetAllocator::<u8>::new(&used, &failed);
        let mut words = BudgetAllocator::<u32>::new(&used, &failed);
        let allocation = bytes.alloc_cell(8);
        assert_eq!(allocation.slice().len(), 8);
        assert_eq!(words.alloc_cell(3).slice().len(), 0);
        assert!(failed.get());
        bytes.free_cell(allocation);
        assert_eq!(used.get(), BROTLI_WORKSPACE as usize - 16);
        let allocation = words.alloc_cell(4);
        assert_eq!(allocation.slice().len(), 4);
        words.free_cell(allocation);
    }
}
