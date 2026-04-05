use std::{env, fs, process};

use shuck_parser::parser::Lexer;

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: dump_tokens <path> [start_line] [end_line]");
        process::exit(2);
    };

    let start_line = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let end_line = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(start_line + 20);

    let input = fs::read_to_string(&path).unwrap_or_else(|err| {
        eprintln!("failed to read {path}: {err}");
        process::exit(1);
    });

    let mut lexer = Lexer::new(&input);
    while let Some(token) = lexer.next_spanned_token() {
        if token.span.start.line >= start_line && token.span.start.line <= end_line {
            println!(
                "{:>6}:{:<4} {:<30?} {:?}",
                token.span.start.line,
                token.span.start.column,
                token.span,
                token.token
            );
        }
    }
}
