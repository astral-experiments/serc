//! Startup state for the pinned, uncustomized CPython command-line callers.

use std::path::Path;
use std::sync::OnceLock;

use serc::{InitialInternState, PythonVersion};

#[path = "compileall_profiles_data.rs"]
mod data;

/// Reuse each caller's state without sharing state accumulated by compiled files.
pub(super) fn for_path(
    version: PythonVersion,
    optimization: usize,
    path: &Path,
) -> InitialInternState {
    static PROFILES: [OnceLock<InitialInternState>; 24] = [const { OnceLock::new() }; 24];
    let target = match version {
        PythonVersion::Py312 => 0,
        PythonVersion::Py313 => 1,
        PythonVersion::Py314 => 2,
        PythonVersion::Py315 => 3,
    };
    let caller = usize::from(
        !path
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().ends_with(b".py")),
    );
    let index = target * 6 + caller * 3 + optimization.min(2);
    let bit = 1 << index;
    PROFILES[index]
        .get_or_init(|| {
            InitialInternState::from_partial_with_absence(
                data::RETAINED
                    .iter()
                    .filter_map(|(name, profiles)| (profiles & bit != 0).then_some(*name)),
                move |target, value| {
                    target == version
                        && !value.contains('/')
                        && data::UNCERTAIN
                            .binary_search_by_key(&value, |(name, _)| *name)
                            .map_or(true, |index| data::UNCERTAIN[index].1 & bit == 0)
                },
            )
        })
        .clone()
}
