use shuck_ast::{Command, CompoundCommand, Redirect, Script, Word};

use crate::command::{FormatCommand, FormatCompoundCommand};
use crate::redirect::FormatRedirect;
use crate::script::FormatScript;
use crate::shared_traits::{AsFormat, FormatOwnedWithRule, FormatRefWithRule, IntoFormat};
use crate::word::FormatWord;

impl<'a> AsFormat<'a> for Script {
    type Format = FormatRefWithRule<'a, Script, FormatScript>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatScript)
    }
}

impl<'a> IntoFormat<'a> for Script {
    type Format = FormatOwnedWithRule<Script, FormatScript>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatScript)
    }
}

impl<'a> AsFormat<'a> for Command {
    type Format = FormatRefWithRule<'a, Command, FormatCommand>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatCommand)
    }
}

impl<'a> IntoFormat<'a> for Command {
    type Format = FormatOwnedWithRule<Command, FormatCommand>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatCommand)
    }
}

impl<'a> AsFormat<'a> for CompoundCommand {
    type Format = FormatRefWithRule<'a, CompoundCommand, FormatCompoundCommand>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatCompoundCommand)
    }
}

impl<'a> IntoFormat<'a> for CompoundCommand {
    type Format = FormatOwnedWithRule<CompoundCommand, FormatCompoundCommand>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatCompoundCommand)
    }
}

impl<'a> AsFormat<'a> for Word {
    type Format = FormatRefWithRule<'a, Word, FormatWord>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatWord)
    }
}

impl<'a> IntoFormat<'a> for Word {
    type Format = FormatOwnedWithRule<Word, FormatWord>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatWord)
    }
}

impl<'a> AsFormat<'a> for Redirect {
    type Format = FormatRefWithRule<'a, Redirect, FormatRedirect>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatRedirect)
    }
}

impl<'a> IntoFormat<'a> for Redirect {
    type Format = FormatOwnedWithRule<Redirect, FormatRedirect>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatRedirect)
    }
}
