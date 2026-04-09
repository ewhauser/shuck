use std::{env, process};

use shuck_benchmark::{benchmark_cases, parse_fixture_with_benchmark_counters};
use shuck_parser::parser::ParserBenchmarkCounters;

fn main() {
    let case_name = env::args().nth(1).unwrap_or_else(|| "nvm".to_string());
    let cases = benchmark_cases();
    let Some(case) = cases.iter().find(|case| case.name == case_name) else {
        eprintln!("unknown benchmark case `{case_name}`");
        eprintln!("available cases:");
        for case in &cases {
            eprintln!("  {}", case.name);
        }
        process::exit(2);
    };

    let mut counters = ParserBenchmarkCounters::default();
    let mut recovered_files = 0usize;

    for file in case.files {
        let counted = parse_fixture_with_benchmark_counters(file.source);
        counters.lexer_current_position_calls += counted.counters.lexer_current_position_calls;
        counters.parser_set_current_spanned_calls +=
            counted.counters.parser_set_current_spanned_calls;
        counters.parser_advance_raw_calls += counted.counters.parser_advance_raw_calls;
        recovered_files += usize::from(counted.recovered);
    }

    println!("case: {}", case.name);
    println!("files: {}", case.files.len());
    println!("recovered_files: {recovered_files}");
    println!(
        "lexer_current_position_calls: {}",
        counters.lexer_current_position_calls
    );
    println!(
        "parser_set_current_spanned_calls: {}",
        counters.parser_set_current_spanned_calls
    );
    println!(
        "parser_advance_raw_calls: {}",
        counters.parser_advance_raw_calls
    );
}
