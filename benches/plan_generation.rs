use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use assert_fs::TempDir;
use remove_except::build_plan;

const GROUPS: usize = 32;
const FILES_PER_GROUP: usize = 64;
const LARGE_GROUPS: usize = 16;
const LARGE_FILES_PER_GROUP: usize = 512;
const DEEP_LEVELS: usize = 24;

struct BenchmarkCase<'a> {
    name: &'a str,
    root: &'a Path,
    patterns: Vec<String>,
    item_count: usize,
}

fn main() {
    let (samples, iterations) = parse_args();
    let (wide_fixture, wide_item_count) = create_wide_fixture();
    let (large_fixture, large_item_count) =
        create_wide_fixture_with(LARGE_GROUPS, LARGE_FILES_PER_GROUP);
    let (deep_fixture, deep_item_count) = create_deep_fixture();

    let cases = [
        BenchmarkCase {
            name: "wide_sparse_single_glob",
            root: wide_fixture.path(),
            patterns: vec!["**/item-000.txt".to_owned()],
            item_count: wide_item_count,
        },
        BenchmarkCase {
            name: "wide_many_globs",
            root: wide_fixture.path(),
            patterns: (0..GROUPS / 2)
                .map(|group| format!("group-{group:02}/*.txt"))
                .collect(),
            item_count: wide_item_count,
        },
        BenchmarkCase {
            name: "wide_literal_directory",
            root: wide_fixture.path(),
            patterns: vec!["group-00".to_owned()],
            item_count: wide_item_count,
        },
        BenchmarkCase {
            name: "wide_all_items_kept",
            root: wide_fixture.path(),
            patterns: vec![".".to_owned()],
            item_count: wide_item_count,
        },
        BenchmarkCase {
            name: "large_sparse_single_glob",
            root: large_fixture.path(),
            patterns: vec!["**/item-000.txt".to_owned()],
            item_count: large_item_count,
        },
        BenchmarkCase {
            name: "large_all_items_kept",
            root: large_fixture.path(),
            patterns: vec![".".to_owned()],
            item_count: large_item_count,
        },
        BenchmarkCase {
            name: "deep_sparse_match",
            root: deep_fixture.path(),
            patterns: vec![format!("{}/item.txt", vec!["d"; DEEP_LEVELS].join("/"))],
            item_count: deep_item_count,
        },
    ];

    println!("Plan generation benchmark: {samples} samples, {iterations} iterations/sample");
    for case in cases {
        run_case(&case, samples, iterations);
    }
}

fn create_wide_fixture() -> (TempDir, usize) {
    create_wide_fixture_with(GROUPS, FILES_PER_GROUP)
}

fn create_wide_fixture_with(groups: usize, files_per_group: usize) -> (TempDir, usize) {
    let fixture = TempDir::new().expect("create wide benchmark fixture");
    for group in 0..groups {
        let directory = fixture.path().join(format!("group-{group:02}"));
        fs::create_dir(&directory).expect("create benchmark group");
        for file in 0..files_per_group {
            fs::write(directory.join(format!("item-{file:03}.txt")), b"x")
                .expect("create benchmark file");
        }
    }
    (fixture, groups * (files_per_group + 1))
}

fn create_deep_fixture() -> (TempDir, usize) {
    let fixture = TempDir::new().expect("create deep benchmark fixture");
    let mut directory = fixture.path().to_path_buf();
    for _ in 0..DEEP_LEVELS {
        directory.push("d");
        fs::create_dir(&directory).expect("create benchmark directory");
        fs::write(directory.join("item.txt"), b"x").expect("create benchmark file");
    }
    (fixture, DEEP_LEVELS * 2)
}

fn run_case(case: &BenchmarkCase<'_>, samples: usize, iterations: usize) {
    let _ = build_plan(case.root, &case.patterns).expect("warm up plan generation");
    let mut per_iteration = Vec::with_capacity(samples);

    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..iterations {
            let plan = build_plan(case.root, &case.patterns).expect("generate removal plan");
            std::hint::black_box(plan);
        }
        per_iteration.push(start.elapsed() / u32::try_from(iterations).unwrap());
    }

    per_iteration.sort_unstable();
    let median = per_iteration[per_iteration.len() / 2];
    let items_per_second = case.item_count as f64 / median.as_secs_f64();
    println!(
        "{}: {} items, {} patterns, median {:.3} ms/plan ({:.0} items/s)",
        case.name,
        case.item_count,
        case.patterns.len(),
        median.as_secs_f64() * 1_000.0,
        items_per_second
    );
    print_sample_range(&per_iteration);
}

fn print_sample_range(samples: &[Duration]) {
    println!(
        "  sample range: {:.3}..{:.3} ms/plan",
        samples[0].as_secs_f64() * 1_000.0,
        samples[samples.len() - 1].as_secs_f64() * 1_000.0
    );
}

fn parse_args() -> (usize, usize) {
    let mut samples = 9;
    let mut iterations = 3;
    let mut args = std::env::args().skip(1);

    while let Some(argument) = args.next() {
        if argument == "--bench" {
            continue;
        }
        let value = args
            .next()
            .unwrap_or_else(|| panic!("missing value for {argument}"));
        let parsed = value
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("invalid value for {argument}: {value}"));
        assert!(parsed > 0, "{argument} must be greater than zero");

        match argument.as_str() {
            "--samples" => samples = parsed,
            "--iterations" => iterations = parsed,
            _ => panic!("unknown argument: {argument}"),
        }
    }

    (samples, iterations)
}
