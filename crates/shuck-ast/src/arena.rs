use std::marker::PhantomData;

/// A compact typed index into an arena-backed store.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Idx<T> {
    raw: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Idx<T> {
    /// Creates an index from a `usize`, panicking when it does not fit in `u32`.
    pub fn new(index: usize) -> Self {
        let raw =
            u32::try_from(index).unwrap_or_else(|err| panic!("arena index must fit in u32: {err}"));
        Self::from_raw(raw)
    }

    /// Creates an index from its raw representation.
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    /// Returns this index as a `usize` for slice access.
    pub const fn index(self) -> usize {
        self.raw as usize
    }

    /// Returns this index as its packed raw value.
    pub const fn raw(self) -> u32 {
        self.raw
    }
}

impl<T> Clone for Idx<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Idx<T> {}

/// A compact typed contiguous range into an arena-backed store.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct IdRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> IdRange<T> {
    /// Returns an empty range.
    pub const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    /// Creates a range from a typed start index and length.
    pub fn new(start: Idx<T>, len: usize) -> Self {
        Self::from_start_len(start.index(), len)
    }

    /// Creates a range from untyped start and length values.
    pub fn from_start_len(start: usize, len: usize) -> Self {
        let end = start
            .checked_add(len)
            .unwrap_or_else(|| panic!("arena range end must not overflow usize"));
        if end > u32::MAX as usize {
            panic!("arena range end must fit in u32");
        }
        let start = u32::try_from(start)
            .unwrap_or_else(|err| panic!("arena range start must fit in u32: {err}"));
        let len = u32::try_from(len)
            .unwrap_or_else(|err| panic!("arena range length must fit in u32: {err}"));
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }

    /// Returns the first index in the range.
    pub const fn start(self) -> Idx<T> {
        Idx::from_raw(self.start)
    }

    /// Returns the start offset as a `usize`.
    pub const fn start_index(self) -> usize {
        self.start as usize
    }

    /// Returns the end offset as a `usize`.
    pub const fn end_index(self) -> usize {
        self.start as usize + self.len as usize
    }

    /// Returns the number of items in the range.
    pub const fn len(self) -> usize {
        self.len as usize
    }

    /// Returns `true` when the range contains no items.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Returns the slice covered by this range.
    pub fn slice(self, store: &[T]) -> &[T] {
        &store[self.start_index()..self.end_index()]
    }

    /// Returns the mutable slice covered by this range.
    pub fn slice_mut(self, store: &mut [T]) -> &mut [T] {
        &mut store[self.start_index()..self.end_index()]
    }
}

impl<T> Clone for IdRange<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for IdRange<T> {}

impl<T> Default for IdRange<T> {
    fn default() -> Self {
        Self::empty()
    }
}

/// A simple append-only store for variable-length typed child lists.
#[derive(Debug, Clone, Default)]
pub struct ListArena<T> {
    items: Vec<T>,
}

impl<T> ListArena<T> {
    /// Creates an empty list arena.
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Creates an empty list arena with capacity for `capacity` items.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
        }
    }

    /// Appends one item and returns its typed index.
    pub fn push(&mut self, item: T) -> Idx<T> {
        let id = Idx::new(self.items.len());
        self.items.push(item);
        self.check_len();
        id
    }

    /// Appends a variable-length list and returns the typed range for it.
    pub fn push_many<I>(&mut self, items: I) -> IdRange<T>
    where
        I: IntoIterator<Item = T>,
    {
        let start = self.items.len();
        self.items.extend(items);
        self.check_len();
        IdRange::from_start_len(start, self.items.len() - start)
    }

    /// Returns all arena items as a slice.
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// Returns all arena items as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.items
    }

    /// Returns the slice covered by `range`.
    pub fn get(&self, range: IdRange<T>) -> &[T] {
        range.slice(&self.items)
    }

    /// Returns the mutable slice covered by `range`.
    pub fn get_mut(&mut self, range: IdRange<T>) -> &mut [T] {
        range.slice_mut(&mut self.items)
    }

    /// Returns the number of items in the arena.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` when the arena is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Consumes the arena and returns the packed items.
    pub fn into_vec(self) -> Vec<T> {
        self.items
    }

    /// Consumes the arena and returns the packed items as a boxed slice.
    pub fn into_boxed_slice(self) -> Box<[T]> {
        self.items.into_boxed_slice()
    }

    fn check_len(&self) {
        if self.items.len() > u32::MAX as usize {
            panic!("arena length must fit in u32");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IdRange, Idx, ListArena};

    #[test]
    fn typed_index_round_trips_raw_values() {
        let id = Idx::<String>::new(42);
        assert_eq!(id.index(), 42);
        assert_eq!(id.raw(), 42);
        assert_eq!(Idx::<String>::from_raw(7).index(), 7);
    }

    #[test]
    #[should_panic(expected = "arena index must fit in u32")]
    fn typed_index_panics_when_out_of_bounds() {
        let _ = Idx::<()>::new(u32::MAX as usize + 1);
    }

    #[test]
    fn typed_ranges_slice_storage() {
        let values = [10, 20, 30, 40];
        let range = IdRange::<i32>::from_start_len(1, 2);
        assert_eq!(range.start().index(), 1);
        assert_eq!(range.len(), 2);
        assert_eq!(range.slice(&values), &[20, 30]);
    }

    #[test]
    fn empty_ranges_are_zero_length() {
        let range = IdRange::<i32>::empty();
        let values = [10, 20];
        assert!(range.is_empty());
        assert_eq!(range.slice(&values), &[]);
    }

    #[test]
    #[should_panic(expected = "arena range end must fit in u32")]
    fn typed_range_panics_when_end_is_out_of_bounds() {
        let _ = IdRange::<()>::from_start_len(u32::MAX as usize, 1);
    }

    #[test]
    fn list_arena_packs_variable_length_lists() {
        let mut arena = ListArena::new();
        let first = arena.push_many([1, 2]);
        let empty = arena.push_many([]);
        let second = arena.push_many([3, 4, 5]);

        assert_eq!(arena.get(first), &[1, 2]);
        assert_eq!(arena.get(empty), &[]);
        assert_eq!(arena.get(second), &[3, 4, 5]);
        assert_eq!(arena.as_slice(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn list_arena_mutates_ranges_in_place() {
        let mut arena = ListArena::new();
        let range = arena.push_many([1, 2, 3]);
        arena.get_mut(range)[1] = 9;
        assert_eq!(arena.get(range), &[1, 9, 3]);
    }
}
