#![feature(const_generics)]
#![allow(incomplete_features)]

use std::num::NonZeroUsize;

/// A `GenerationIndex` is a composite key into the contiguous block of memory which is managed
/// by our `GenerationalArena`. It is an `index` into the contiguous block of memory with an
/// associated `generation` information.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub struct GenerationIndex {
    index: SlotIndex,
    generation: GenerationCounter,
}

type SlotIndex = usize;

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
struct GenerationCounter(NonZeroUsize);

impl GenerationCounter {
    fn new() -> GenerationCounter {
        GenerationCounter(NonZeroUsize::new(1).unwrap())
    }

    fn next_generation(&mut self) {
        self.0 = NonZeroUsize::new(
            self.0
                .get()
                .checked_add(1)
                .expect("exhausted generation counter"),
        )
        .unwrap();
    }
}

/// A `Slot<T>` represents a region in the arena that is large enough to hold exactly one of `T`.
///
/// A `Slot` can be either:
///
/// - `Free`: no previous data occupied this slot, can be trivially inserted in-place.
/// - `Occupied`: the slot is already occupied and must be freed before new data can be inserted.
#[derive(Debug, Clone)]
enum Slot<T> {
    Free {
        next_free: Option<SlotIndex>,
    },
    Occupied {
        generation: GenerationCounter,
        value: T,
    },
}

/// A generational arena is a managed memory pool for managing the allocation and deallocation of
/// a homogenous data type. The arena consists of multiple *slots*, and we track which slots are
/// "empty". When inserting data, we also track a `GenerationIndex` which is used to track the age
/// of the inserted data. When a slot is freed, the generation is incremented to differentiate
/// between data inserted at different times. The arena is allocated on the heap.
///
/// Advantages:
///
/// - Reduce likelihood of free entity indices (can reuse existing indicies).
/// - Allows safe mutable access and deletion (old generation signals absence).
/// - Constant memory usage: `std::mem::size_of(T) * ELEMENTS_COUNT` plus two tracking fields.
///
/// Disadvantages:
///
/// - Memory bloat due to unoccupied `Free` slots.
#[derive(Debug, Clone)]
pub struct GenerationalArena<T, const ELEMENTS_COUNT: usize>
where
    T: Clone,
{
    items: Vec<Slot<T>>,
    free_list_head: Option<SlotIndex>,
    generation: GenerationCounter,
    len: usize,
}

impl<T, const ELEMENTS_COUNT: usize> GenerationalArena<T, ELEMENTS_COUNT>
where
    T: Clone,
{
    #[inline]
    pub fn new() -> GenerationalArena<T, ELEMENTS_COUNT> {
        assert!(ELEMENTS_COUNT > 0);

        let mut arena = GenerationalArena {
            items: Vec::with_capacity(ELEMENTS_COUNT),
            free_list_head: None,
            generation: GenerationCounter::new(),
            len: 0,
        };

        arena.initialize_slots();
        arena
    }

    fn initialize_slots(&mut self) {
        self.items.extend((0..ELEMENTS_COUNT).map(|i| {
            // The last slot's `next_free == None` indicates that we have no more free space.
            if i == ELEMENTS_COUNT - 1 {
                Slot::Free { next_free: None }
            } else {
                Slot::Free {
                    next_free: Some(i + 1),
                }
            }
        }));

        self.free_list_head = Some(0);
    }

    #[inline]
    pub fn try_insert(&mut self, value: T) -> Result<GenerationIndex, T> {
        match self.free_list_head {
            None => {
                // We've exceeded our full capacity, so we return ownership of `T` back to the
                // caller.
                Err(value)
            }
            Some(i) => match self.items[i] {
                Slot::Occupied { .. } => {
                    // This cannot happen, unless the free list is corrupted.
                    panic!("corrupt free list");
                }
                Slot::Free { next_free } => {
                    self.free_list_head = next_free;
                    self.len += 1;

                    let gen_index = GenerationIndex {
                        index: i,
                        generation: self.generation,
                    };

                    self.items[gen_index.index] = Slot::Occupied {
                        generation: self.generation,
                        value,
                    };

                    Ok(gen_index)
                }
            },
        }
    }

    #[inline]
    pub fn remove(&mut self, generation_index: GenerationIndex) -> Option<T> {
        assert!(generation_index.index < ELEMENTS_COUNT);

        match self.items[generation_index.index] {
            Slot::Occupied { generation, .. } if generation_index.generation == generation => {
                let slot = std::mem::replace(
                    &mut self.items[generation_index.index],
                    Slot::Free {
                        next_free: self.free_list_head,
                    },
                );

                self.generation.next_generation();
                self.free_list_head = Some(generation_index.index);
                self.len -= 1;

                match slot {
                    Slot::Occupied {
                        generation: _,
                        value,
                    } => Some(value),
                    _ => unreachable!(),
                }
            }
            _ => None,
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.len
    }

    // Note: it's unfortunate that the code for `get` and `get_mut` are identical expect for the
    // mutability of `self`, and transitively, `generation` and `value`. It looks like Higher-Kinded
    // Types (HKT) is needed in order to be parametric over the mutability of `self`.
    #[inline]
    pub fn get(&self, generation_index: GenerationIndex) -> Option<&T> {
        match self.items.get(generation_index.index) {
            Some(Slot::Occupied { generation, value })
                if *generation == generation_index.generation =>
            {
                Some(value)
            }
            _ => None,
        }
    }

    #[inline]
    pub fn get_mut(&mut self, generation_index: GenerationIndex) -> Option<&mut T> {
        match self.items.get_mut(generation_index.index) {
            Some(Slot::Occupied { generation, value })
                if *generation == generation_index.generation =>
            {
                Some(value)
            }
            _ => None,
        }
    }

    #[inline]
    pub fn contains(&self, generation_index: GenerationIndex) -> bool {
        self.get(generation_index).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_get_live_value() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        let i = arena.try_insert(42).unwrap();
        assert_eq!(arena.remove(i).unwrap(), 42);
        assert!(!arena.contains(i));
    }

    #[test]
    fn cannot_get_free_value() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        let i = arena.try_insert(42).unwrap();
        assert_eq!(arena.remove(i).unwrap(), 42);
        assert!(!arena.contains(i));
    }

    #[test]
    fn cannot_get_other_generation_value() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        let i = arena.try_insert(42).unwrap();
        assert_eq!(arena.remove(i).unwrap(), 42);
        assert!(!arena.contains(i));
        let j = arena.try_insert(42).unwrap();
        assert!(!arena.contains(i));
        assert_eq!(*arena.get(j).unwrap(), 42);
        assert!(i != j);
    }

    #[test]
    fn try_insert_on_full_should_err() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        arena.try_insert(42).unwrap();
        assert_eq!(arena.try_insert(42).unwrap_err(), 42);
    }

    #[test]
    fn try_insert_with_indicies_match() {
        let mut arena = GenerationalArena::<u32, 3>::new();
        let a = arena.try_insert(40).ok().unwrap();
        let b = arena.try_insert(41).ok().unwrap();
        let c = arena.try_insert(42).ok().unwrap();
        assert!(
            a.generation == b.generation
                && b.generation == c.generation
                && a.generation == c.generation
        );
        assert_eq!(*arena.get(a).unwrap(), 40);
        assert_eq!(*arena.get(b).unwrap(), 41);
        assert_eq!(*arena.get(c).unwrap(), 42);
    }


    #[test]
    fn get_mut() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        let i = arena.try_insert(5).unwrap();
        *arena.get_mut(i).unwrap() += 1;
        assert_eq!(*arena.get(i).unwrap(), 6);
    }

    #[test]
    #[should_panic]
    fn index_deleted_item() {
        let mut arena = GenerationalArena::<u32, 1>::new();
        let idx = arena.try_insert(42).unwrap();
        eprintln!("{:#?}", idx);
        arena.remove(idx);
        arena.get(idx).unwrap();
    }
}
