//! File-driven marshal comparisons, preserving each suite's CPython oracle and source bytes.

use datatest_stable::Utf8Path;
use serc::{OptimizationLevel, PythonVersion};

#[allow(dead_code)]
mod common;
use common::cpython;
#[allow(dead_code)]
#[path = "common/optimization.rs"]
mod optimization;
#[path = "common/private_type_parameters.rs"]
mod private_type_parameters;

const OPTIMIZATION_ROOT: &str = "tests/fixtures/optimization";

/// Compare a fixture with CPython 3.14 at O0, using its basename as the compiled filename.
fn cpython(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = fs_err::read_to_string(path)?;
    let filename = path.file_name().expect("fixture has a filename");
    common::assert_matches_cpython_marshal(&source, filename);
    Ok(())
}

/// Read the version/level matrix and logical filename from the optimization fixture path.
fn optimized(path: &Utf8Path) -> datatest_stable::Result<()> {
    let relative = path.strip_prefix(OPTIMIZATION_ROOT)?;
    let mut components = relative.components();
    let versions = components
        .next()
        .ok_or("missing Python versions")?
        .as_str()
        .split('+')
        .map(str::parse::<PythonVersion>)
        .collect::<Result<Vec<_>, _>>()?;
    let levels = components
        .next()
        .ok_or("missing optimization levels")?
        .as_str()
        .split('+')
        .map(str::parse::<OptimizationLevel>)
        .collect::<Result<Vec<_>, _>>()?;
    // The next two directories identify the original test and its individual source case.
    components.nth(1).ok_or("missing test or case directory")?;
    let filename = components
        .map(|component| component.as_str())
        .collect::<Vec<_>>()
        .join("/");
    if filename.is_empty() {
        return Err("missing compiled filename".into());
    }
    let source = fs_err::read_to_string(path)?;
    optimization::assert_matches_cpython_versions(&source, &filename, &levels, &versions);
    Ok(())
}

/// Retain the private-type-parameter oracle's fixed filename and four Python targets.
fn private_types(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = fs_err::read_to_string(path)?;
    let name = path
        .parent()
        .and_then(Utf8Path::file_name)
        .expect("fixture has a case directory");
    for version in cpython::VERSIONS {
        private_type_parameters::assert_matches_cpython(version, name, &source);
    }
    Ok(())
}

datatest_stable::harness! {
    { test = cpython, root = "tests/fixtures/compatibility", pattern = r"\.py$" },
    { test = optimized, root = OPTIMIZATION_ROOT },
    { test = private_types, root = "tests/fixtures/private_type_parameters", pattern = r"\.py$" },
}
