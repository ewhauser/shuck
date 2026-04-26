use std::path::Path;

use shuck_extract::{EmbeddedScript, ExpressionTaint, ExtractedDialect, extract_all};

const WORKFLOW_EDGE_CASES: &str = include_str!("fixtures/github-actions-edge-cases.yml");
const COMPOSITE_ACTION: &str = include_str!("fixtures/github-actions-composite-action.yml");

#[test]
fn extracts_workflow_edge_case_fixture() {
    let scripts = extract_all(
        Path::new(".github/workflows/edge-cases.yml"),
        WORKFLOW_EDGE_CASES,
    )
    .unwrap();

    assert_eq!(scripts.len(), 26);

    let missing_unix = script(&scripts, "jobs.missing-unix.steps[0].run");
    assert_eq!(missing_unix.dialect, ExtractedDialect::Bash);
    assert!(missing_unix.implicit_flags.errexit);
    assert!(missing_unix.implicit_flags.pipefail);
    assert_eq!(
        missing_unix.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -eo pipefail {0}")
    );
    assert_eq!(missing_unix.source, "echo \"$WORKFLOW_VALUE\"\n");

    let defaulted = script(&scripts, "jobs.defaulted.steps[0].run");
    assert_eq!(defaulted.dialect, ExtractedDialect::Sh);
    assert!(defaulted.implicit_flags.errexit);
    assert!(!defaulted.implicit_flags.pipefail);
    assert_eq!(
        defaulted.implicit_flags.template.as_deref(),
        Some("sh -e {0}")
    );

    let bash = script(&scripts, "jobs.explicit-shells.steps[0].run");
    assert_eq!(bash.dialect, ExtractedDialect::Bash);
    assert_eq!(bash.source, "echo \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(bash.placeholders[0].expression, "github.sha");
    assert_eq!(bash.placeholders[0].taint, ExpressionTaint::Trusted);

    let sh = script(&scripts, "jobs.explicit-shells.steps[1].run");
    assert_eq!(sh.dialect, ExtractedDialect::Sh);
    assert_eq!(sh.source, "echo \"$STEP_VALUE\"\n");

    let custom = script(&scripts, "jobs.explicit-shells.steps[2].run");
    assert_eq!(custom.dialect, ExtractedDialect::Bash);
    assert!(custom.implicit_flags.errexit);
    assert!(custom.implicit_flags.pipefail);
    assert_eq!(
        custom.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -e -o pipefail {0}")
    );
    assert_eq!(custom.source, "printf '%s\\n' \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(
        custom.placeholders[0].expression,
        "github.event.pull_request.title"
    );
    assert_eq!(
        custom.placeholders[0].taint,
        ExpressionTaint::UserControlled
    );

    let pwsh = script(&scripts, "jobs.explicit-shells.steps[3].run");
    assert_eq!(pwsh.dialect, ExtractedDialect::Unsupported);

    let windows_default = script(&scripts, "jobs.windows.steps[0].run");
    assert_eq!(windows_default.dialect, ExtractedDialect::Unsupported);

    let windows_shell = script(&scripts, "jobs.windows.steps[1].run");
    assert_eq!(windows_shell.dialect, ExtractedDialect::Unsupported);

    let windows_bash = script(&scripts, "jobs.windows.steps[2].run");
    assert_eq!(windows_bash.dialect, ExtractedDialect::Bash);

    let anchored = script(&scripts, "jobs.anchors-and-env.steps[0].run");
    assert_eq!(anchored.dialect, ExtractedDialect::Bash);
    assert_eq!(anchored.source, "echo \"$WORKFLOW_VALUE\"\n");

    let env_alias = script(&scripts, "jobs.anchors-and-env.steps[1].run");
    assert_eq!(env_alias.dialect, ExtractedDialect::Bash);
    assert_eq!(env_alias.source, "echo \"${_SHUCK_GHA_1}\"\n");
    assert_eq!(env_alias.placeholders[0].expression, "env.WORKFLOW_VALUE");
    assert_eq!(env_alias.placeholders[0].taint, ExpressionTaint::Trusted);

    let anchored_block = script(&scripts, "jobs.anchors-and-env.steps[2].run");
    assert_eq!(anchored_block.dialect, ExtractedDialect::Bash);
    assert!(anchored_block.source.contains("*prod) echo prod ;;"));

    let inline_glob = script(&scripts, "jobs.anchors-and-env.steps[3].run");
    assert_eq!(inline_glob.dialect, ExtractedDialect::Bash);
    assert_eq!(inline_glob.source, "rm -- *.tmp");

    let comma_glob = script(&scripts, "jobs.anchors-and-env.steps[4].run");
    assert_eq!(comma_glob.dialect, ExtractedDialect::Bash);
    assert_eq!(comma_glob.source, "echo foo,*.tmp");

    let brace_glob = script(&scripts, "jobs.anchors-and-env.steps[5].run");
    assert_eq!(brace_glob.dialect, ExtractedDialect::Bash);
    assert_eq!(brace_glob.source, "echo {*.tmp,*.log}");

    let colon_pattern = script(&scripts, "jobs.anchors-and-env.steps[6].run");
    assert_eq!(colon_pattern.dialect, ExtractedDialect::Bash);
    assert_eq!(colon_pattern.source, "echo foo:*bar");

    let short_alias_default = script(&scripts, "jobs.short-shell-aliases.steps[0].run");
    assert_eq!(short_alias_default.dialect, ExtractedDialect::Bash);
    assert!(short_alias_default.implicit_flags.errexit);
    assert!(short_alias_default.implicit_flags.pipefail);
    assert_eq!(
        short_alias_default.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -e -o pipefail {0}")
    );

    let short_alias_unsupported = script(&scripts, "jobs.short-shell-aliases.steps[1].run");
    assert_eq!(
        short_alias_unsupported.dialect,
        ExtractedDialect::Unsupported
    );

    let short_alias_comment = script(&scripts, "jobs.short-shell-aliases.steps[2].run");
    assert_eq!(short_alias_comment.dialect, ExtractedDialect::Bash);
    assert!(short_alias_comment.implicit_flags.errexit);
    assert!(short_alias_comment.implicit_flags.pipefail);
    assert_eq!(
        short_alias_comment.implicit_flags.template.as_deref(),
        Some("bash --noprofile --norc -e -o pipefail {0}")
    );

    let reused_before = script(&scripts, "jobs.redefined-anchors.steps[0].run");
    assert_eq!(reused_before.dialect, ExtractedDialect::Sh);
    assert_eq!(reused_before.source, "echo reused sh");

    let redefined = script(&scripts, "jobs.redefined-anchors.steps[1].run");
    assert_eq!(redefined.dialect, ExtractedDialect::Bash);
    assert_eq!(redefined.source, "echo redefine bash");

    let reused_after = script(&scripts, "jobs.redefined-anchors.steps[2].run");
    assert_eq!(reused_after.dialect, ExtractedDialect::Bash);
    assert_eq!(reused_after.source, "echo reused bash");

    let flow_style = script(&scripts, "jobs.flow-style.steps[0].run");
    assert_eq!(flow_style.dialect, ExtractedDialect::Sh);
    assert_eq!(flow_style.source, "echo flow");
    assert_eq!(flow_style.host_start_column, 37);

    let flow_alias = script(&scripts, "jobs.flow-style.steps[1].run");
    assert_eq!(flow_alias.dialect, ExtractedDialect::Sh);
    assert_eq!(flow_alias.source, "echo flow alias");

    let comment_alias = script(&scripts, "jobs.flow-style.steps[2].run");
    assert_eq!(comment_alias.dialect, ExtractedDialect::Sh);
    assert_eq!(comment_alias.source, "echo comment alias");

    let flow_pwsh = script(&scripts, "jobs.flow-style.steps[3].run");
    assert_eq!(flow_pwsh.dialect, ExtractedDialect::Unsupported);
}

#[test]
fn extracts_composite_action_edge_case_fixture() {
    let scripts = extract_all(Path::new("action.yml"), COMPOSITE_ACTION).unwrap();

    assert_eq!(scripts.len(), 3);

    let bash = script(&scripts, "runs.steps[0].run");
    assert_eq!(bash.dialect, ExtractedDialect::Bash);
    assert_eq!(
        bash.source,
        "echo \"$COMPOSITE_VALUE\"\necho \"${_SHUCK_GHA_1}\"\n"
    );
    assert_eq!(bash.placeholders[0].expression, "github.sha");
    assert_eq!(bash.placeholders[0].taint, ExpressionTaint::Trusted);

    let sh = script(&scripts, "runs.steps[1].run");
    assert_eq!(sh.dialect, ExtractedDialect::Sh);
    assert_eq!(sh.source, "echo \"${_SHUCK_GHA_1}\"");
    assert_eq!(sh.placeholders[0].expression, "inputs.message");
    assert_eq!(sh.placeholders[0].taint, ExpressionTaint::Unknown);

    let unsupported = script(&scripts, "runs.steps[2].run");
    assert_eq!(unsupported.dialect, ExtractedDialect::Unsupported);
}

fn script<'a>(scripts: &'a [EmbeddedScript], label: &str) -> &'a EmbeddedScript {
    scripts
        .iter()
        .find(|script| script.label == label)
        .unwrap_or_else(|| panic!("missing extracted script `{label}`"))
}
