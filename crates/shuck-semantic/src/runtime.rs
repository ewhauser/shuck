use shuck_ast::{Name, Word};

const COMMON_PREINITIALIZED: &[&str] = &[
    "IFS",
    "USER",
    "HOME",
    "SHELL",
    "PWD",
    "TERM",
    "LANG",
    "SUDO_USER",
    "DOAS_USER",
];

const BASH_PREINITIALIZED: &[&str] = &[
    "LINENO",
    "FUNCNAME",
    "BASH_SOURCE",
    "BASH_LINENO",
    "RANDOM",
    "BASH_REMATCH",
    "READLINE_LINE",
    "BASH_VERSION",
    "BASH_VERSINFO",
    "OSTYPE",
    "HISTCONTROL",
    "HISTSIZE",
    "COMP_WORDS",
    "COMP_CWORD",
];

const ALWAYS_USED_BINDINGS: &[&str] = &["IFS"];
const BASH_ALWAYS_USED_BINDINGS: &[&str] = &["COMPREPLY"];
const EMPTY_IMPLICIT_READS: &[&str] = &[];
const READ_IMPLICIT_READS: &[&str] = &["IFS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimePrelude {
    bash_enabled: bool,
    common_preinitialized: &'static [&'static str],
    bash_preinitialized: &'static [&'static str],
    always_used_bindings: &'static [&'static str],
    bash_always_used_bindings: &'static [&'static str],
}

impl RuntimePrelude {
    pub(crate) fn new(bash_enabled: bool) -> Self {
        Self {
            bash_enabled,
            common_preinitialized: COMMON_PREINITIALIZED,
            bash_preinitialized: BASH_PREINITIALIZED,
            always_used_bindings: ALWAYS_USED_BINDINGS,
            bash_always_used_bindings: BASH_ALWAYS_USED_BINDINGS,
        }
    }

    pub(crate) fn bash_enabled(&self) -> bool {
        self.bash_enabled
    }

    pub(crate) fn is_preinitialized(&self, name: &Name) -> bool {
        contains_name(self.common_preinitialized, name)
            || (self.bash_enabled && contains_name(self.bash_preinitialized, name))
    }

    pub(crate) fn is_always_used_binding(&self, name: &Name) -> bool {
        contains_name(self.always_used_bindings, name)
            || (self.bash_enabled && contains_name(self.bash_always_used_bindings, name))
    }

    pub(crate) fn implicit_reads_for_simple_command(
        &self,
        command_name: &Name,
        _args: &[Word],
        _source: &str,
    ) -> &'static [&'static str] {
        match command_name.as_str() {
            "read" => READ_IMPLICIT_READS,
            _ => EMPTY_IMPLICIT_READS,
        }
    }
}

fn contains_name(names: &[&str], name: &Name) -> bool {
    names.iter().any(|candidate| *candidate == name.as_str())
}
