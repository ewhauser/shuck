use shuck_ast::Name;

const COMMON_PREINITIALIZED: &[&str] = &[
    "IFS",
    "OPTIND",
    "OPTARG",
    "OPTERR",
    "USER",
    "HOME",
    "HOSTNAME",
    "SHELL",
    "PWD",
    "TERM",
    "PATH",
    "CDPATH",
    "LD_LIBRARY_PATH",
    "LANG",
    "SUDO_USER",
    "DOAS_USER",
    "BASH_ENV",
    "BASH_XTRACEFD",
    "ENV",
    "INPUTRC",
    "MAIL",
    "OLDPWD",
    "PS1",
    "PS2",
    "PS4",
    "PROMPT_DIRTRIM",
    "SECONDS",
    "TIMEFORMAT",
    "TMOUT",
    "COMPREPLY",
];

const BASH_PREINITIALIZED: &[&str] = &[
    "BASH_ALIASES",
    "BASH_ARGC",
    "BASH_ARGV",
    "BASH_CMDS",
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
    "HISTFILE",
    "HISTFILESIZE",
    "HISTIGNORE",
    "HISTSIZE",
    "HISTTIMEFORMAT",
    "PIPESTATUS",
    "DIRSTACK",
    "GROUPS",
    "MAPFILE",
    "COLUMNS",
    "PROMPT_COMMAND",
    "PS3",
    "READLINE_POINT",
    "COMP_WORDBREAKS",
    "COMP_WORDS",
    "COMP_CWORD",
    "COMPREPLY",
    "COPROC",
];

const BASH_PREINITIALIZED_ARRAYS: &[&str] = &[
    "BASH_ALIASES",
    "BASH_ARGC",
    "BASH_ARGV",
    "BASH_CMDS",
    "BASH_LINENO",
    "BASH_REMATCH",
    "BASH_SOURCE",
    "BASH_VERSINFO",
    "COMP_WORDS",
    "COMPREPLY",
    "COPROC",
    "DIRSTACK",
    "FUNCNAME",
    "GROUPS",
    "MAPFILE",
    "PIPESTATUS",
];

const ALWAYS_USED_BINDINGS: &[&str] = &["IFS", "PATH", "CDPATH", "COMPREPLY", "FLAGS_PARENT"];
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
            || is_locale_binding(name)
            || (self.bash_enabled && contains_name(self.bash_preinitialized, name))
    }

    pub(crate) fn is_preinitialized_array(&self, name: &Name) -> bool {
        self.bash_enabled && contains_name(BASH_PREINITIALIZED_ARRAYS, name)
    }

    pub(crate) fn is_always_used_binding(&self, name: &Name) -> bool {
        self.is_preinitialized(name)
            || contains_name(self.always_used_bindings, name)
            || is_locale_binding(name)
            || (self.bash_enabled && contains_name(self.bash_always_used_bindings, name))
    }

    pub(crate) fn implicit_reads_for_simple_command(
        &self,
        command_name: &Name,
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

fn is_locale_binding(name: &Name) -> bool {
    let name = name.as_str();
    name == "LANG" || name.starts_with("LC_")
}
