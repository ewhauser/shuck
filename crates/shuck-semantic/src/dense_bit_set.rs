#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DenseBitSet {
    words: Vec<usize>,
}

impl DenseBitSet {
    pub(crate) const WORD_BITS: usize = usize::BITS as usize;

    pub(crate) fn new(bit_len: usize) -> Self {
        Self {
            words: vec![0; bit_len.div_ceil(Self::WORD_BITS)],
        }
    }

    pub(crate) fn insert(&mut self, index: usize) {
        let word = index / Self::WORD_BITS;
        let bit = index % Self::WORD_BITS;
        self.words[word] |= 1usize << bit;
    }

    pub(crate) fn remove(&mut self, index: usize) {
        let word = index / Self::WORD_BITS;
        let bit = index % Self::WORD_BITS;
        self.words[word] &= !(1usize << bit);
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        let word = index / Self::WORD_BITS;
        let bit = index % Self::WORD_BITS;
        self.words
            .get(word)
            .is_some_and(|word| (word & (1usize << bit)) != 0)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub(crate) fn clear(&mut self) {
        self.words.fill(0);
    }

    pub(crate) fn as_words(&self) -> &[usize] {
        &self.words
    }

    pub(crate) fn copy_from(&mut self, other: &Self) {
        self.copy_from_words(&other.words);
    }

    pub(crate) fn copy_from_words(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words.len(), words.len());
        self.words.copy_from_slice(words);
    }

    pub(crate) fn replace_if_changed(&mut self, other: &Self) -> bool {
        self.replace_if_changed_words(&other.words)
    }

    pub(crate) fn replace_if_changed_words(&mut self, words: &[usize]) -> bool {
        debug_assert_eq!(self.words.len(), words.len());
        if self.words == words {
            false
        } else {
            self.copy_from_words(words);
            true
        }
    }

    pub(crate) fn union_with(&mut self, other: &Self) {
        self.union_with_words(&other.words);
    }

    pub(crate) fn union_with_words(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words.len(), words.len());
        for (word, other_word) in self.words.iter_mut().zip(words) {
            *word |= *other_word;
        }
    }

    pub(crate) fn subtract_with(&mut self, other: &Self) {
        self.subtract_with_words(&other.words);
    }

    pub(crate) fn subtract_with_words(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words.len(), words.len());
        for (word, other_word) in self.words.iter_mut().zip(words) {
            *word &= !*other_word;
        }
    }

    pub(crate) fn intersect_with(&mut self, other: &Self) {
        self.intersect_with_words(&other.words);
    }

    pub(crate) fn intersect_with_words(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words.len(), words.len());
        for (word, other_word) in self.words.iter_mut().zip(words) {
            *word &= *other_word;
        }
    }

    pub(crate) fn iter_ones(&self) -> DenseBitSetIter<'_> {
        DenseBitSetIter {
            words: &self.words,
            word_index: 0,
            current_word: 0,
        }
    }
}

/// A flat 2D bitset stored as a single backing buffer. Replaces
/// `Vec<DenseBitSet>` patterns where every row has the same bit length, so
/// per-row clones do not allocate independent `Vec<usize>` storage.
#[derive(Debug, Clone)]
pub(crate) struct DenseBitMatrix {
    words_per_row: usize,
    rows: usize,
    data: Vec<usize>,
}

impl DenseBitMatrix {
    pub(crate) fn zeros(rows: usize, bit_len: usize) -> Self {
        let words_per_row = bit_len.div_ceil(DenseBitSet::WORD_BITS);
        Self {
            words_per_row,
            rows,
            data: vec![0; rows * words_per_row],
        }
    }

    pub(crate) fn rows(&self) -> usize {
        self.rows
    }

    pub(crate) fn row(&self, row: usize) -> &[usize] {
        let start = row * self.words_per_row;
        &self.data[start..start + self.words_per_row]
    }

    pub(crate) fn fill_row_from_words(&mut self, row: usize, src: &[usize]) {
        debug_assert_eq!(src.len(), self.words_per_row);
        let start = row * self.words_per_row;
        self.data[start..start + self.words_per_row].copy_from_slice(src);
    }

    pub(crate) fn fill_all_rows_from_words(&mut self, src: &[usize]) {
        debug_assert_eq!(src.len(), self.words_per_row);
        for row in 0..self.rows {
            let start = row * self.words_per_row;
            self.data[start..start + self.words_per_row].copy_from_slice(src);
        }
    }

    pub(crate) fn insert(&mut self, row: usize, index: usize) {
        let word = index / DenseBitSet::WORD_BITS;
        let bit = index % DenseBitSet::WORD_BITS;
        self.data[row * self.words_per_row + word] |= 1usize << bit;
    }

    pub(crate) fn contains(&self, row: usize, index: usize) -> bool {
        let word = index / DenseBitSet::WORD_BITS;
        let bit = index % DenseBitSet::WORD_BITS;
        self.data
            .get(row * self.words_per_row + word)
            .is_some_and(|word| (word & (1usize << bit)) != 0)
    }

    /// Replace `row` with `src` if they differ. Returns whether the row changed.
    pub(crate) fn replace_row_if_changed(&mut self, row: usize, src: &[usize]) -> bool {
        debug_assert_eq!(src.len(), self.words_per_row);
        let start = row * self.words_per_row;
        let target = &mut self.data[start..start + self.words_per_row];
        if target == src {
            false
        } else {
            target.copy_from_slice(src);
            true
        }
    }
}

pub(crate) struct DenseBitSetIter<'a> {
    words: &'a [usize],
    word_index: usize,
    current_word: usize,
}

impl Iterator for DenseBitSetIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_word != 0 {
                let bit = self.current_word.trailing_zeros() as usize;
                self.current_word &= self.current_word - 1;
                return Some((self.word_index - 1) * DenseBitSet::WORD_BITS + bit);
            }

            let next_word = self.words.get(self.word_index).copied()?;
            self.current_word = next_word;
            self.word_index += 1;
        }
    }
}
