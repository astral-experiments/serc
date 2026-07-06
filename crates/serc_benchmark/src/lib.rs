pub mod criterion;

#[derive(Clone, Copy, Debug)]
pub struct Workload {
    pub id: &'static str,
    pub source: &'static str,
    pub filename: &'static str,
}

pub const WORKLOADS: &[Workload] = &[
    Workload {
        id: "context_managers",
        source: include_str!("../../serc/tests/fixtures/execution/context_managers.py"),
        filename: "benchmark/context_managers.py",
    },
    Workload {
        id: "annotation_scopes",
        source: include_str!("../../serc/tests/fixtures/benchmark/annotation_scopes.py"),
        filename: "annotation_scopes.py",
    },
    Workload {
        id: "match_control_flow",
        source: include_str!("../../serc/tests/fixtures/benchmark/match_control_flow.py"),
        filename: "match.py",
    },
    Workload {
        id: "loop_async_edges",
        source: include_str!("../../serc/tests/fixtures/benchmark/loop_async_edges.py"),
        filename: "loop_tail.py",
    },
];
