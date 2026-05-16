use shuck_ast::{Command, CompoundCommand, File, Redirect, Stmt, Word};

use crate::command::{FormatCommand, FormatCompoundCommand, FormatStatement};
use crate::redirect::FormatRedirect;
use crate::script::FormatFile;
use crate::shared_traits::{AsFormat, FormatOwnedWithRule, FormatRefWithRule, IntoFormat};
use crate::word::FormatWord;

impl<'a> AsFormat<'a> for File {
    type Format = FormatRefWithRule<'a, File, FormatFile>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatFile)
    }
}

impl<'a> IntoFormat<'a> for File {
    type Format = FormatOwnedWithRule<File, FormatFile>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatFile)
    }
}

impl<'a> AsFormat<'a> for Stmt {
    type Format = FormatRefWithRule<'a, Stmt, FormatStatement>;

    fn format(&'a self) -> Self::Format {
        FormatRefWithRule::new(self, FormatStatement)
    }
}

impl<'a> IntoFormat<'a> for Stmt {
    type Format = FormatOwnedWithRule<Stmt, FormatStatement>;

    fn into_format(self) -> Self::Format {
        FormatOwnedWithRule::new(self, FormatStatement)
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
