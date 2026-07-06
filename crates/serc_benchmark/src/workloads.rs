#[derive(Clone, Copy, Debug)]
pub struct Workload {
    pub id: &'static str,
    pub source: &'static str,
    pub filename: &'static str,
}

impl Workload {
    const fn new(id: &'static str, source: &'static str, filename: &'static str) -> Self {
        Self {
            id,
            source,
            filename,
        }
    }
}

pub const COMPILE_MICRO_WORKLOADS: &[Workload] = &[
    Workload::new(
        "micro_baseline",
        include_str!("../../serc/tests/fixtures/benchmark/micro/baseline.py"),
        "benchmark/micro/baseline.py",
    ),
    Workload::new(
        "micro_literals",
        include_str!("../../serc/tests/fixtures/benchmark/micro/literals.py"),
        "benchmark/micro/literals.py",
    ),
    Workload::new(
        "micro_operations",
        include_str!("../../serc/tests/fixtures/benchmark/micro/operations.py"),
        "benchmark/micro/operations.py",
    ),
    Workload::new(
        "micro_calls",
        include_str!("../../serc/tests/fixtures/benchmark/micro/calls.py"),
        "benchmark/micro/calls.py",
    ),
    Workload::new(
        "micro_functions",
        include_str!("../../serc/tests/fixtures/benchmark/micro/functions.py"),
        "benchmark/micro/functions.py",
    ),
    Workload::new(
        "micro_classes",
        include_str!("../../serc/tests/fixtures/benchmark/micro/classes.py"),
        "benchmark/micro/classes.py",
    ),
    Workload::new(
        "micro_branches",
        include_str!("../../serc/tests/fixtures/benchmark/micro/branches.py"),
        "benchmark/micro/branches.py",
    ),
    Workload::new(
        "micro_loops",
        include_str!("../../serc/tests/fixtures/benchmark/micro/loops.py"),
        "benchmark/micro/loops.py",
    ),
    Workload::new(
        "micro_exceptions",
        include_str!("../../serc/tests/fixtures/benchmark/micro/exceptions.py"),
        "benchmark/micro/exceptions.py",
    ),
    Workload::new(
        "micro_with",
        include_str!("../../serc/tests/fixtures/benchmark/micro/with.py"),
        "benchmark/micro/with.py",
    ),
    Workload::new(
        "micro_comprehensions",
        include_str!("../../serc/tests/fixtures/benchmark/micro/comprehensions.py"),
        "benchmark/micro/comprehensions.py",
    ),
    Workload::new(
        "micro_generators",
        include_str!("../../serc/tests/fixtures/benchmark/micro/generators.py"),
        "benchmark/micro/generators.py",
    ),
    Workload::new(
        "micro_pattern_matching",
        include_str!("../../serc/tests/fixtures/benchmark/micro/pattern_matching.py"),
        "benchmark/micro/pattern_matching.py",
    ),
    Workload::new(
        "micro_async",
        include_str!("../../serc/tests/fixtures/benchmark/micro/async.py"),
        "benchmark/micro/async.py",
    ),
    Workload::new(
        "micro_annotations",
        include_str!("../../serc/tests/fixtures/benchmark/micro/annotations.py"),
        "benchmark/micro/annotations.py",
    ),
    Workload::new(
        "micro_imports",
        include_str!("../../serc/tests/fixtures/benchmark/micro/imports.py"),
        "benchmark/micro/imports.py",
    ),
];

pub const COMPOSITE_WORKLOADS: &[Workload] = &[
    Workload::new(
        "context_managers",
        include_str!("../../serc/tests/fixtures/execution/context_managers.py"),
        "benchmark/context_managers.py",
    ),
    Workload::new(
        "annotation_scopes",
        include_str!("../../serc/tests/fixtures/benchmark/annotation_scopes.py"),
        "annotation_scopes.py",
    ),
    Workload::new(
        "match_control_flow",
        include_str!("../../serc/tests/fixtures/benchmark/match_control_flow.py"),
        "match.py",
    ),
    Workload::new(
        "loop_async_edges",
        include_str!("../../serc/tests/fixtures/benchmark/loop_async_edges.py"),
        "loop_tail.py",
    ),
];

pub const LARGE_WORKLOADS: &[Workload] = &[Workload::new(
    "large_dataset",
    include_str!("../../serc/tests/fixtures/benchmark/large/dataset.py"),
    "benchmark/large/dataset.py",
)];

pub const MARSHAL_MICRO_WORKLOADS: &[Workload] = &[
    Workload::new(
        "constants",
        include_str!("../../serc/tests/fixtures/benchmark/marshal/constants.py"),
        "benchmark/marshal/constants.py",
    ),
    Workload::new(
        "nested_code",
        include_str!("../../serc/tests/fixtures/benchmark/marshal/nested_code.py"),
        "benchmark/marshal/nested_code.py",
    ),
];
