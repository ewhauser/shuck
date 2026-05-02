use crate::Rule;

const WORD_COUNT: usize = Rule::COUNT.div_ceil(64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleSet([u64; WORD_COUNT]);

impl RuleSet {
    pub const EMPTY: Self = Self([0; WORD_COUNT]);

    pub fn all() -> Self {
        Rule::iter().collect()
    }

    pub const fn from_rules(rules: &[Rule]) -> Self {
        let mut words = [0; WORD_COUNT];
        let mut index = 0;
        while index < rules.len() {
            let rule = rules[index] as usize;
            let word = rule / 64;
            let bit = rule % 64;
            words[word] |= 1u64 << bit;
            index += 1;
        }
        Self(words)
    }

    pub const fn contains(&self, rule: Rule) -> bool {
        let word = (rule as usize) / 64;
        let bit = (rule as usize) % 64;
        (self.0[word] & (1u64 << bit)) != 0
    }

    pub fn insert(&mut self, rule: Rule) {
        let word = (rule as usize) / 64;
        let bit = (rule as usize) % 64;
        self.0[word] |= 1u64 << bit;
    }

    pub fn remove(&mut self, rule: Rule) {
        let word = (rule as usize) / 64;
        let bit = (rule as usize) % 64;
        self.0[word] &= !(1u64 << bit);
    }

    pub fn set(&mut self, rule: Rule, enabled: bool) {
        if enabled {
            self.insert(rule);
        } else {
            self.remove(rule);
        }
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut words = [0; WORD_COUNT];
        let mut index = 0;
        while index < WORD_COUNT {
            words[index] = self.0[index] | other.0[index];
            index += 1;
        }
        Self(words)
    }

    pub fn subtract(&self, other: &Self) -> Self {
        let mut words = [0; WORD_COUNT];
        let mut index = 0;
        while index < WORD_COUNT {
            words[index] = self.0[index] & !other.0[index];
            index += 1;
        }
        Self(words)
    }

    pub fn iter(&self) -> impl Iterator<Item = Rule> + '_ {
        Rule::iter().filter(|rule| self.contains(*rule))
    }

    pub fn len(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl FromIterator<Rule> for RuleSet {
    fn from_iter<T: IntoIterator<Item = Rule>>(iter: T) -> Self {
        let mut set = Self::EMPTY;
        for rule in iter {
            set.insert(rule);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_insert_remove_and_iteration() {
        let mut set = RuleSet::EMPTY;
        assert!(set.is_empty());

        set.insert(Rule::UnusedAssignment);
        assert!(set.contains(Rule::UnusedAssignment));
        assert_eq!(set.len(), 1);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![Rule::UnusedAssignment]);

        set.remove(Rule::UnusedAssignment);
        assert!(set.is_empty());
    }

    #[test]
    fn can_be_built_in_const_context() {
        const RULES: RuleSet = RuleSet::from_rules(&[Rule::UnusedAssignment, Rule::BareRead]);

        assert_eq!(
            RULES.iter().collect::<Vec<_>>(),
            vec![Rule::UnusedAssignment, Rule::BareRead]
        );
    }
}
