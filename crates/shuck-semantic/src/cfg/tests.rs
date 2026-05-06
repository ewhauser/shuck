use crate::SemanticModel;
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;

fn model(source: &str) -> SemanticModel {
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    SemanticModel::build(&output.file, source, &indexer)
}

#[test]
fn recorded_program_preserves_logical_list_order_in_ranges() {
    let source = "a && b || c\n";
    let model = model(source);

    let lists = model.list_commands();
    assert_eq!(lists.len(), 1);
    let segments = &lists[0].segments;

    assert_eq!(segments.len(), 3);
    assert!(segments[0].command_span.slice(source).starts_with("a"));
    assert!(segments[1].command_span.slice(source).starts_with("b"));
    assert!(segments[2].command_span.slice(source).starts_with("c"));
}

#[test]
fn flattened_logical_lists_preserve_short_circuit_cfg_paths() {
    let source = "true && true || exit 1\nprintf '%s\\n' reachable\n";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn recorded_program_preserves_pipeline_segment_order_in_ranges() {
    let source = "a | b | c\n";
    let model = model(source);

    let pipelines = model.pipeline_commands();
    assert_eq!(pipelines.len(), 1);
    let segments = &pipelines[0].segments;

    assert_eq!(segments.len(), 3);
    assert!(segments[0].command_span.slice(source).starts_with("a"));
    assert!(segments[1].command_span.slice(source).starts_with("b"));
    assert!(segments[2].command_span.slice(source).starts_with("c"));
}

#[test]
fn recorded_program_and_cfg_capture_non_arithmetic_var_ref_nested_regions() {
    let source = "\
[[ -v assoc[\"$(printf inner)\"] ]]
echo done
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);

    let file_commands = model
        .recorded_program
        .commands_in(model.recorded_program().file_commands());
    assert_eq!(file_commands.len(), 2);
    let conditional = model.recorded_program().command(file_commands[0]);
    let nested_regions = model
        .recorded_program
        .nested_regions(conditional.nested_regions);
    assert_eq!(nested_regions.len(), 1);
    let nested_commands = model
        .recorded_program
        .commands_in(nested_regions[0].commands);
    assert_eq!(nested_commands.len(), 1);
    let nested = model.recorded_program().command(nested_commands[0]);
    assert_eq!(nested.span.slice(source), "printf inner");

    let analysis = model.analysis();
    let cfg = analysis.cfg();

    assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
    assert!(!cfg.block_ids_for_span(nested.span).is_empty());
    assert!(
        cfg.blocks()
            .iter()
            .flat_map(|block| block.commands.iter())
            .any(|span| span.slice(source) == "printf inner")
    );
}

#[test]
fn recorded_program_and_cfg_capture_arithmetic_var_ref_nested_regions() {
    let source = "\
[[ -v assoc[$(( $(printf inner) + 1 ))] ]]
echo done
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);

    let file_commands = model
        .recorded_program
        .commands_in(model.recorded_program().file_commands());
    assert_eq!(file_commands.len(), 2);
    let conditional = model.recorded_program().command(file_commands[0]);
    let nested_regions = model
        .recorded_program
        .nested_regions(conditional.nested_regions);
    assert_eq!(nested_regions.len(), 1);
    let nested_commands = model
        .recorded_program
        .commands_in(nested_regions[0].commands);
    assert_eq!(nested_commands.len(), 1);
    let nested = model.recorded_program().command(nested_commands[0]);
    assert_eq!(nested.span.slice(source), "printf inner");

    let analysis = model.analysis();
    let cfg = analysis.cfg();

    assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
    assert!(!cfg.block_ids_for_span(nested.span).is_empty());
    assert!(
        cfg.blocks()
            .iter()
            .flat_map(|block| block.commands.iter())
            .any(|span| span.slice(source) == "printf inner")
    );
}
