#[cfg(all(codspeed, feature = "codspeed"))]
pub use ::codspeed_criterion_compat::*;
#[cfg(all(not(codspeed), feature = "benchmarks"))]
pub use ::criterion::*;
