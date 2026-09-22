/// A named Python source workload.
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
        include_str!("../resources/micro/baseline.py"),
        "benchmark/micro/baseline.py",
    ),
    Workload::new(
        "micro_literals",
        include_str!("../resources/micro/literals.py"),
        "benchmark/micro/literals.py",
    ),
    Workload::new(
        "micro_operations",
        include_str!("../resources/micro/operations.py"),
        "benchmark/micro/operations.py",
    ),
    Workload::new(
        "micro_calls",
        include_str!("../resources/micro/calls.py"),
        "benchmark/micro/calls.py",
    ),
    Workload::new(
        "micro_functions",
        include_str!("../resources/micro/functions.py"),
        "benchmark/micro/functions.py",
    ),
    Workload::new(
        "micro_classes",
        include_str!("../resources/micro/classes.py"),
        "benchmark/micro/classes.py",
    ),
    Workload::new(
        "micro_branches",
        include_str!("../resources/micro/branches.py"),
        "benchmark/micro/branches.py",
    ),
    Workload::new(
        "micro_loops",
        include_str!("../resources/micro/loops.py"),
        "benchmark/micro/loops.py",
    ),
    Workload::new(
        "micro_exceptions",
        include_str!("../resources/micro/exceptions.py"),
        "benchmark/micro/exceptions.py",
    ),
    Workload::new(
        "micro_with",
        include_str!("../resources/micro/with.py"),
        "benchmark/micro/with.py",
    ),
    Workload::new(
        "micro_comprehensions",
        include_str!("../resources/micro/comprehensions.py"),
        "benchmark/micro/comprehensions.py",
    ),
    Workload::new(
        "micro_generators",
        include_str!("../resources/micro/generators.py"),
        "benchmark/micro/generators.py",
    ),
    Workload::new(
        "micro_pattern_matching",
        include_str!("../resources/micro/pattern_matching.py"),
        "benchmark/micro/pattern_matching.py",
    ),
    Workload::new(
        "micro_async",
        include_str!("../resources/micro/async.py"),
        "benchmark/micro/async.py",
    ),
    Workload::new(
        "micro_annotations",
        include_str!("../resources/micro/annotations.py"),
        "benchmark/micro/annotations.py",
    ),
    Workload::new(
        "micro_imports",
        include_str!("../resources/micro/imports.py"),
        "benchmark/micro/imports.py",
    ),
];

pub const CONTEXT_MANAGERS: &str = include_str!("../resources/context_managers.py");
pub const ANNOTATION_SCOPES: &str = include_str!("../resources/annotation_scopes.py");
pub const MATCH_CONTROL_FLOW: &str = include_str!("../resources/match_control_flow.py");
pub const LOOP_ASYNC_EDGES: &str = include_str!("../resources/loop_async_edges.py");

pub const COMPOSITE_WORKLOADS: &[Workload] = &[
    Workload::new(
        "context_managers",
        CONTEXT_MANAGERS,
        "benchmark/context_managers.py",
    ),
    Workload::new(
        "annotation_scopes",
        ANNOTATION_SCOPES,
        "annotation_scopes.py",
    ),
    Workload::new("match_control_flow", MATCH_CONTROL_FLOW, "match.py"),
    Workload::new("loop_async_edges", LOOP_ASYNC_EDGES, "loop_tail.py"),
];

pub const LARGE_WORKLOADS: &[Workload] = &[Workload::new(
    "large_dataset",
    include_str!("../resources/large/dataset.py"),
    "benchmark/large/dataset.py",
)];

pub const MARSHAL_MICRO_WORKLOADS: &[Workload] = &[
    Workload::new(
        "constants",
        include_str!("../resources/marshal/constants.py"),
        "benchmark/marshal/constants.py",
    ),
    Workload::new(
        "nested_code",
        include_str!("../resources/marshal/nested_code.py"),
        "benchmark/marshal/nested_code.py",
    ),
];
