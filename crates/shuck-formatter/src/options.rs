use std::path::Path;

use shuck_format::{FormatOptions, IndentStyle, LineEnding, PrinterOptions};
use shuck_parser::ShellDialect as ParseDialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    #[default]
    Auto,
    Bash,
    Posix,
    Mksh,
    Zsh,
}

impl ShellDialect {
    #[must_use]
    pub fn resolve(self, source: &str, path: Option<&Path>) -> ParseDialect {
        match self {
            Self::Auto => infer_dialect(source, path),
            Self::Bash => ParseDialect::Bash,
            Self::Posix => ParseDialect::Posix,
            Self::Mksh => ParseDialect::Mksh,
            Self::Zsh => ParseDialect::Zsh,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFormatOptions {
    dialect: ShellDialect,
    indent_style: IndentStyle,
    indent_width: u8,
    binary_next_line: bool,
    switch_case_indent: bool,
    space_redirects: bool,
    keep_padding: bool,
    function_next_line: bool,
    never_split: bool,
    simplify: bool,
    minify: bool,
}

impl Default for ShellFormatOptions {
    fn default() -> Self {
        Self {
            dialect: ShellDialect::Auto,
            indent_style: IndentStyle::Tab,
            indent_width: 8,
            binary_next_line: false,
            switch_case_indent: false,
            space_redirects: false,
            keep_padding: false,
            function_next_line: false,
            never_split: false,
            simplify: false,
            minify: false,
        }
    }
}

macro_rules! option_getters {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(&self) -> $ty {
                self.$field
            }
        )+
    };
}

macro_rules! option_builders {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(mut self, value: $ty) -> Self {
                self.$field = value;
                self
            }
        )+
    };
}

macro_rules! resolved_option_getters {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(&self) -> $ty {
                self.options.$field
            }
        )+
    };
}

impl ShellFormatOptions {
    option_getters! {
        dialect: dialect -> ShellDialect;
        indent_style: indent_style -> IndentStyle;
        indent_width: indent_width -> u8;
        binary_next_line: binary_next_line -> bool;
        switch_case_indent: switch_case_indent -> bool;
        space_redirects: space_redirects -> bool;
        keep_padding: keep_padding -> bool;
        function_next_line: function_next_line -> bool;
        never_split: never_split -> bool;
        simplify: simplify -> bool;
        minify: minify -> bool;
    }

    option_builders! {
        with_dialect: dialect -> ShellDialect;
        with_indent_style: indent_style -> IndentStyle;
        with_binary_next_line: binary_next_line -> bool;
        with_switch_case_indent: switch_case_indent -> bool;
        with_space_redirects: space_redirects -> bool;
        with_keep_padding: keep_padding -> bool;
        with_function_next_line: function_next_line -> bool;
        with_never_split: never_split -> bool;
        with_simplify: simplify -> bool;
        with_minify: minify -> bool;
    }

    #[must_use]
    pub fn with_indent_width(mut self, indent_width: u8) -> Self {
        self.indent_width = indent_width.max(1);
        self
    }

    #[must_use]
    pub fn resolve(&self, source: &str, path: Option<&Path>) -> ResolvedShellFormatOptions {
        let mut options = self.clone();
        options.indent_width = options.indent_width.max(1);
        ResolvedShellFormatOptions {
            dialect: self.dialect.resolve(source, path),
            options,
            line_ending: detect_line_ending(source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShellFormatOptions {
    options: ShellFormatOptions,
    dialect: ParseDialect,
    line_ending: LineEnding,
}

impl ResolvedShellFormatOptions {
    option_getters! {
        dialect: dialect -> ParseDialect;
    }

    resolved_option_getters! {
        indent_style: indent_style -> IndentStyle;
        indent_width: indent_width -> u8;
    }

    pub(crate) fn indent_unit_columns(&self) -> usize {
        match self.indent_style() {
            IndentStyle::Tab => 1,
            IndentStyle::Space => usize::from(self.indent_width()),
        }
    }

    pub(crate) fn indent_columns(&self, levels: usize) -> usize {
        levels * self.indent_unit_columns()
    }

    pub(crate) fn push_indent_units(&self, target: &mut String, levels: usize) {
        self.push_indent_columns(target, self.indent_columns(levels));
    }

    pub(crate) fn push_indent_columns(&self, target: &mut String, columns: usize) {
        let ch = match self.indent_style() {
            IndentStyle::Tab => '\t',
            IndentStyle::Space => ' ',
        };
        target.extend(std::iter::repeat_n(ch, columns));
    }

    pub(crate) fn indent_prefix(&self, levels: usize) -> String {
        let mut prefix = String::new();
        self.push_indent_units(&mut prefix, levels);
        prefix
    }

    resolved_option_getters! {
        binary_next_line: binary_next_line -> bool;
        switch_case_indent: switch_case_indent -> bool;
        space_redirects: space_redirects -> bool;
        keep_padding: keep_padding -> bool;
        function_next_line: function_next_line -> bool;
        never_split: never_split -> bool;
        simplify: simplify -> bool;
        minify: minify -> bool;
    }

    #[must_use]
    pub fn compact_layout(&self) -> bool {
        self.minify() || self.never_split()
    }

    option_getters! {
        line_ending: line_ending -> LineEnding;
    }
}

impl FormatOptions for ResolvedShellFormatOptions {
    fn as_print_options(&self) -> PrinterOptions {
        PrinterOptions {
            indent_style: self.indent_style(),
            indent_width: self.indent_width(),
            line_width: 80,
            line_ending: self.line_ending,
        }
    }
}

fn infer_dialect(source: &str, path: Option<&Path>) -> ParseDialect {
    if let Some(first_line) = source.lines().next()
        && let Some(interpreter) = shuck_parser::shebang::interpreter_name(first_line)
    {
        return ParseDialect::from_name(interpreter);
    }

    path.and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(ParseDialect::from_name)
        .unwrap_or(ParseDialect::Bash)
}

fn detect_line_ending(source: &str) -> LineEnding {
    if source.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn zsh_extension_resolves_to_zsh_dialect() {
        let resolved = ShellFormatOptions::default()
            .resolve("print ${(m)name}\n", Some(Path::new("script.zsh")));

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }

    #[test]
    fn zsh_shebang_resolves_to_zsh_dialect() {
        let resolved = ShellFormatOptions::default().resolve(
            "#!/bin/zsh\nprint ${(m)name}\n",
            Some(Path::new("script.sh")),
        );

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }

    #[test]
    fn explicit_zsh_dialect_overrides_path_inference() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Zsh);
        let resolved = options.resolve("print ${(m)name}\n", Some(Path::new("script.sh")));

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }
}
