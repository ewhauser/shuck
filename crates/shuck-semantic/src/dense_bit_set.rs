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

    pub(crate) fn copy_from(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        self.words.copy_from_slice(&other.words);
    }

    pub(crate) fn replace_if_changed(&mut self, other: &Self) -> bool {
        debug_assert_eq!(self.words.len(), other.words.len());
        if self.words == other.words {
            false
        } else {
            self.copy_from(other);
            true
        }
    }

    pub(crate) fn union_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
            *word |= *other_word;
        }
    }

    pub(crate) fn subtract_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
            *word &= !*other_word;
        }
    }

    pub(crate) fn intersect_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
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
