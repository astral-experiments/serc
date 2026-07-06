pub mod criterion;
mod workloads;

pub use workloads::{
    COMPILE_MICRO_WORKLOADS, COMPOSITE_WORKLOADS, LARGE_WORKLOADS, MARSHAL_MICRO_WORKLOADS,
    Workload,
};
