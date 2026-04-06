use rustc_hash::FxHashSet;
use shuck_ast::{Script, Span};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

use crate::{Diagnostic, Rule, RuleSet, ShellDialect, Violation, rules};

pub struct Checker<'a> {
    semantic: &'a SemanticModel,
    indexer: &'a Indexer,
    script: &'a Script,
    source: &'a str,
    rules: &'a RuleSet,
    shell: ShellDialect,
    diagnostics: Vec<Diagnostic>,
    reported: FxHashSet<DiagnosticKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DiagnosticKey {
    rule: Rule,
    start: usize,
    end: usize,
}

impl DiagnosticKey {
    fn new(rule: Rule, span: Span) -> Self {
        Self {
            rule,
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

impl<'a> Checker<'a> {
    pub fn new(
        script: &'a Script,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        rules: &'a RuleSet,
        shell: ShellDialect,
    ) -> Self {
        Self {
            semantic,
            indexer,
            script,
            source,
            rules,
            shell,
            diagnostics: Vec::new(),
            reported: FxHashSet::default(),
        }
    }

    pub fn semantic(&self) -> &'a SemanticModel {
        self.semantic
    }

    pub fn indexer(&self) -> &'a Indexer {
        self.indexer
    }

    pub fn ast(&self) -> &'a Script {
        self.script
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn is_rule_enabled(&self, rule: Rule) -> bool {
        self.rules.contains(rule)
    }

    pub fn shell(&self) -> ShellDialect {
        self.shell
    }

    pub fn report<V: Violation>(&mut self, violation: V, span: Span) {
        let diagnostic = Diagnostic::new(violation, span);
        self.reported
            .insert(DiagnosticKey::new(diagnostic.rule, diagnostic.span));
        self.diagnostics.push(diagnostic);
    }

    pub fn report_dedup<V: Violation>(&mut self, violation: V, span: Span) {
        let diagnostic = Diagnostic::new(violation, span);
        let key = DiagnosticKey::new(diagnostic.rule, diagnostic.span);
        if !self.reported.insert(key) {
            return;
        }
        self.diagnostics.push(diagnostic);
    }

    pub fn check(mut self) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return self.diagnostics;
        }

        self.check_bindings();
        self.check_references();
        self.check_scopes();
        self.check_declarations();
        self.check_call_sites();
        self.check_source_refs();
        self.check_commands();
        self.check_flow();
        self.diagnostics
    }

    fn check_bindings(&mut self) {
        if self.is_rule_enabled(Rule::UnusedAssignment) {
            rules::correctness::unused_assignment::unused_assignment(self);
        }
    }

    fn check_references(&mut self) {
        if self.is_rule_enabled(Rule::UndefinedVariable) {
            rules::correctness::undefined_variable::undefined_variable(self);
        }
    }

    fn check_scopes(&mut self) {}

    fn check_declarations(&mut self) {
        if self.is_rule_enabled(Rule::LocalTopLevel) {
            rules::correctness::script_scope_local::local_top_level(self);
        }
    }

    fn check_call_sites(&mut self) {
        if self.is_rule_enabled(Rule::OverwrittenFunction) {
            rules::correctness::overwritten_function::overwritten_function(self);
        }
    }

    fn check_source_refs(&mut self) {
        if self.is_rule_enabled(Rule::DynamicSourcePath) {
            rules::correctness::dynamic_source_path::dynamic_source_path(self);
        }
    }

    fn check_commands(&mut self) {
        if self.is_rule_enabled(Rule::UnquotedExpansion) {
            rules::style::unquoted_expansion::unquoted_expansion(self);
        }
        if self.is_rule_enabled(Rule::ReadWithoutRaw) {
            rules::style::read_without_raw::read_without_raw(self);
        }
        if self.is_rule_enabled(Rule::LoopFromCommandOutput) {
            rules::style::loop_from_command_output::loop_from_command_output(self);
        }
        if self.is_rule_enabled(Rule::UnquotedCommandSubstitution) {
            rules::style::unquoted_command_substitution::unquoted_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::LegacyBackticks) {
            rules::style::legacy_backticks::legacy_backticks(self);
        }
        if self.is_rule_enabled(Rule::LegacyArithmeticExpansion) {
            rules::style::legacy_arithmetic_expansion::legacy_arithmetic_expansion(self);
        }
        if self.is_rule_enabled(Rule::PrintfFormatVariable) {
            rules::style::printf_format_variable::printf_format_variable(self);
        }
        if self.is_rule_enabled(Rule::UnquotedArrayExpansion) {
            rules::style::unquoted_array_expansion::unquoted_array_expansion(self);
        }
        if self.is_rule_enabled(Rule::EchoedCommandSubstitution) {
            rules::style::echoed_command_substitution::echoed_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::ExportCommandSubstitution) {
            rules::style::export_command_substitution::export_command_substitution(self);
        }
        if self.is_rule_enabled(Rule::TrapStringExpansion) {
            rules::correctness::trap_string_expansion::trap_string_expansion(self);
        }
        if self.is_rule_enabled(Rule::QuotedBashRegex) {
            rules::correctness::quoted_bash_regex::quoted_bash_regex(self);
        }
        if self.is_rule_enabled(Rule::ChainedTestBranches) {
            rules::correctness::chained_test_branches::chained_test_branches(self);
        }
        if self.is_rule_enabled(Rule::LineOrientedInput) {
            rules::correctness::line_oriented_input::line_oriented_input(self);
        }
        if self.is_rule_enabled(Rule::FindOutputToXargs) {
            rules::correctness::find_output_to_xargs::find_output_to_xargs(self);
        }
        if self.is_rule_enabled(Rule::FindOutputLoop) {
            rules::correctness::find_output_loop::find_output_loop(self);
        }
        if self.is_rule_enabled(Rule::SingleQuotedLiteral) {
            rules::correctness::single_quoted_literal::single_quoted_literal(self);
        }
        if self.is_rule_enabled(Rule::SudoRedirectionOrder) {
            rules::correctness::sudo_redirection_order::sudo_redirection_order(self);
        }
        if self.is_rule_enabled(Rule::ConstantComparisonTest) {
            rules::correctness::constant_comparison_test::constant_comparison_test(self);
        }
        if self.is_rule_enabled(Rule::LoopControlOutsideLoop) {
            rules::correctness::loop_control_outside_loop::loop_control_outside_loop(self);
        }
        if self.is_rule_enabled(Rule::LiteralUnaryStringTest) {
            rules::correctness::literal_unary_string_test::literal_unary_string_test(self);
        }
        if self.is_rule_enabled(Rule::TruthyLiteralTest) {
            rules::correctness::truthy_literal_test::truthy_literal_test(self);
        }
        if self.is_rule_enabled(Rule::ConstantCaseSubject) {
            rules::correctness::constant_case_subject::constant_case_subject(self);
        }
        if self.is_rule_enabled(Rule::EmptyTest) {
            rules::correctness::empty_test::empty_test(self);
        }
        if self.is_rule_enabled(Rule::PipeToKill) {
            rules::correctness::pipe_to_kill::pipe_to_kill(self);
        }
        if self.is_rule_enabled(Rule::PositionalTenBraces) {
            rules::correctness::positional_ten_braces::positional_ten_braces(self);
        }
        if self.is_rule_enabled(Rule::InvalidExitStatus) {
            rules::correctness::invalid_exit_status::invalid_exit_status(self);
        }
        if self.is_rule_enabled(Rule::CasePatternVar) {
            rules::correctness::case_pattern_var::case_pattern_var(self);
        }
        if self.is_rule_enabled(Rule::ArithmeticRedirectionTarget) {
            rules::correctness::arithmetic_redirection_target::arithmetic_redirection_target(self);
        }
        if self.is_rule_enabled(Rule::PatternWithVariable) {
            rules::correctness::pattern_with_variable::pattern_with_variable(self);
        }
        if self.is_rule_enabled(Rule::SubstWithRedirect) {
            rules::correctness::subst_with_redirect::subst_with_redirect(self);
        }
        if self.is_rule_enabled(Rule::SubstWithRedirectErr) {
            rules::correctness::subst_with_redirect_err::subst_with_redirect_err(self);
        }
    }

    fn check_flow(&mut self) {
        if self.is_rule_enabled(Rule::UnreachableAfterExit) {
            rules::correctness::unreachable_after_exit::unreachable_after_exit(self);
        }
    }
}
