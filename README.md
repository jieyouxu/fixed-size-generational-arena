# Homogenous Generational Arena

Learning from <https://www.youtube.com/watch?v=SofC6c9xQv4>. This particular
implementation does not prioritize performance (yet) but instead strives for
soundness and memory safety.

The API design is adapted from
[fitzgen/generational-arena](https://github.com/fitzgen/generational-arena), but
only consits of a subset. Requires nightly `rustc` for const generics.

- **Arena**: request a chunk of memory and perform manual management of the
  requested memory.
- **Generational**: use *generational index* to track free memory in the arena.
- **Homogenous**: all elements stored in the area can only be of the same type.

A **generational area** is commonly used within ECS systems to allocate and
deallocate memory for entities.

## Rationale for Usage in ECS Architectures

In ECS architectures, the *Struct-of-Arrays* (SOA) data organization is
preferred over *Array-of-Structs* for **better cache locality**.

For example, if the game has `Person`s

```rs
struct World {
    people: Vec<Person>,
}
```

Then this has bad cache locality if only a subset or one of `Person`'s attribute
is used (e.g. trying to find the average age of the people).

Instead, for better cache locality, the `Person`'s attributes should be broken
up into smaller *attribute parts*.

```rs
struct World {
    contact_details: Vec<ContactDetails>,
    date_of_births: Vec<Datetime<Utc>>,
}

In memory, `World` is laid out as:

```text
           | Hierarchical    | Struct-of-Arrays
-----------------------------------------------
Cache      | Person0         | Datetime0
Capacity   |                 |-----------------
(e.g. L1   |                 | Datetime1
cache)     |-----------------|-----------------
           | Person 1        | Datetime2
           |                 |-----------------
           |                 | Datetime3
           |-----------------|-----------------
           | ...             | ...
```

## Key Concepts

### Outline

- We keep a *pool* of memory, and remember which slots are "empty" and avaiable
  to store items.
- We track a `generation` when inserting data.
- When reclaiming a slot's memory (freeing), we need to ensure that the
  `generation` greater than the `generation` of the freed slot:

  ```rs
  assert!(input.geneneration > slot.generation)
  ```

### Generational Index

The naive ECS architecture typically refers to an entity by an `usize` index.

To track when and if free space is readily available, or *can* be available by
freeing outdated/old allocated memory, we can upgrade the naive `usize` index
to a `GenerationIndex`. We need to keep track of the `generation` of this entity
index, which keeps track of *when* the entity was placed into the memory space.

When retreiving, we need to check that the item matches the expected generation,
to ensure that if a slot is *reused* (aka memory no longer used is reclaimed,
and the generation is bumped newer), where a different item may be inserted in
place, that we don't get a "dangling index" (effectively analogous to a dangling
reference or use-after-free).

```rs

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct GenerationIndex {
    index: usize,
    generation: usize,
}
```

*A note on `generation`*:

- Typical implementations seem to not handle the possibility of `generation`
  overflowing. That is, typical implementations assume that `GenerationCounter`
  is **monotonically increasing** – but what happens when
  `generation == usize::MAX`?
    - We may force it to be `checked_add` to panic on overflow.

### Slot

We track the "availability" of different "slots" of the arena by encoding it
with the `Slot<T>` enum, for the data type `T`:

```rs
enum Slot<T> {
    Free { next_free: Option<usize> },
    Occupied { generation: usize, value: T }
}
```

A `Slot` can be either:

- `Free`: in which we track the next free slot (or, we may run out of available
  free spaces and become out of memory).
- `Occupied`: slot is in use.
