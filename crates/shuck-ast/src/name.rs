//! Compact owned identifier-like strings for the shell AST.
//!
//! Adapted from Ruff's `ruff_python_ast::name::Name` implementation:
//! `/Users/ewhauser/working/ruff/crates/ruff_python_ast/src/name.rs`

use std::borrow::{Borrow, Cow};
use std::fmt::{Debug, Display, Formatter};
use std::ops::Deref;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Name(compact_str::CompactString);

impl Name {
    #[inline]
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(compact_str::CompactString::new(name))
    }

    #[inline]
    pub const fn new_static(name: &'static str) -> Self {
        Self(compact_str::CompactString::const_new(name))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Debug for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Name({:?})", self.as_str())
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for Name {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for Name {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Borrow<str> for Name {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Name {
    #[inline]
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for Name {
    #[inline]
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

impl From<&String> for Name {
    #[inline]
    fn from(value: &String) -> Self {
        Self(value.into())
    }
}

impl From<Box<str>> for Name {
    #[inline]
    fn from(value: Box<str>) -> Self {
        Self(value.into())
    }
}

impl From<Cow<'_, str>> for Name {
    #[inline]
    fn from(value: Cow<'_, str>) -> Self {
        Self(value.into())
    }
}

impl PartialEq<str> for Name {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<Name> for str {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        other == self
    }
}

impl PartialEq<&str> for Name {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Name> for &str {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        other == self
    }
}

impl PartialEq<String> for Name {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<Name> for String {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        other == self
    }
}

impl PartialEq<&String> for Name {
    #[inline]
    fn eq(&self, other: &&String) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Name> for &String {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        other == self
    }
}

#[cfg(test)]
mod tests {
    use super::Name;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    #[test]
    fn new_and_new_static_match() {
        let dynamic = Name::new("alpha");
        let static_name = Name::new_static("alpha");
        assert_eq!(dynamic, static_name);
        assert_eq!(dynamic.as_str(), "alpha");
    }

    #[test]
    fn compares_against_str_and_string() {
        let name = Name::new("HOME");
        let string = String::from("HOME");

        assert_eq!(name, "HOME");
        assert_eq!("HOME", name);
        assert_eq!(name, string);
        assert_eq!(string, name);
    }

    #[test]
    fn display_and_debug_are_stable() {
        let name = Name::new("select_var");
        assert_eq!(format!("{name}"), "select_var");
        assert_eq!(format!("{name:?}"), "Name(\"select_var\")");
    }

    #[test]
    fn ordering_and_hash_follow_string_contents() {
        let smaller = Name::new("a");
        let larger = Name::new("b");
        assert!(smaller < larger);

        let mut left = DefaultHasher::new();
        smaller.hash(&mut left);
        let mut right = DefaultHasher::new();
        Name::new("a").hash(&mut right);
        assert_eq!(left.finish(), right.finish());
    }

    #[test]
    fn supports_short_and_long_inputs() {
        let short = Name::new("fd");
        let long = Name::new("this_identifier_is_long_enough_to_spill_past_inline_storage");

        assert_eq!(short.as_str(), "fd");
        assert_eq!(
            long.as_str(),
            "this_identifier_is_long_enough_to_spill_past_inline_storage"
        );
    }
}
