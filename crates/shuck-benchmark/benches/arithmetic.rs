use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::configure_benchmark_allocator;
use shuck_parser::parser::Parser;

configure_benchmark_allocator!();

const SAMPLE_SIZE: usize = 50;

#[derive(Debug, Clone)]
struct ArithmeticCase {
    name: &'static str,
    source: String,
}

impl ArithmeticCase {
    fn total_bytes(&self) -> u64 {
        self.source.len() as u64
    }
}

fn arithmetic_case(name: &'static str, snippet: &str, repetitions: usize) -> ArithmeticCase {
    ArithmeticCase {
        name,
        source: repeat_snippet(snippet, repetitions),
    }
}

fn repeat_snippet(snippet: &str, repetitions: usize) -> String {
    let mut source = String::with_capacity(snippet.len() * repetitions);
    for _ in 0..repetitions {
        source.push_str(snippet);
    }
    source
}

fn arithmetic_cases() -> Vec<ArithmeticCase> {
    vec![
        arithmetic_case(
            "double-paren-command",
            "(( ++i ? j-- : (k = 1), m ))\n",
            400,
        ),
        arithmetic_case(
            "arithmetic-expansion",
            "echo \"$((1 + 2))\" \"$[3 + 4]\"\n",
            350,
        ),
        arithmetic_case(
            "subscripts-and-slices",
            "a[i + 1]=x\ndeclare foo[1+2]\necho ${arr[i+1]} ${s:i+1:len*2} ${arr[@]:i:j}\n",
            250,
        ),
        arithmetic_case(
            "arithmetic-for",
            "for (( i = 0 ; i < 10 ; i += ($# - 1))); do echo \"$i\"; done\n",
            180,
        ),
        arithmetic_case("shell-word-operands", "(( \"$(date -u)\" + '3' ))\n", 300),
    ]
}

fn parse_source(source: &str) -> usize {
    let output = Parser::new(black_box(source))
        .parse()
        .expect("arithmetic benchmark inputs should parse");

    black_box(output.script.commands.len())
}

fn bench_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("arithmetic");

    for case in arithmetic_cases() {
        group.sample_size(SAMPLE_SIZE);
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let command_count = parse_source(&case.source);
                black_box(command_count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_arithmetic);
criterion_main!(benches);
