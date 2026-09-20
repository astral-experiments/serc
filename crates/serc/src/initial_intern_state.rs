use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::PythonVersion;
use crate::target::is_preinterned_string_for;

/// Immutable string intern state present before compiling a Python module.
///
/// CPython can reuse strings interned by its bootstrap or earlier imports when it
/// assembles code objects. This can affect marshal object sharing even when code
/// fields compare equal. The default preserves Serc's isolated CPython startup
/// model. An explicit state supplements that model with existing canonical strings.
///
/// Entries describe canonical strings retained throughout compilation. This models
/// a fixed bootstrap, not a long-lived interpreter session in which mortal interned
/// strings may disappear. Compiling an import statement does not execute the import
/// or change this state. Clones share the immutable entries.
#[derive(Clone, Debug, Default)]
pub struct InitialInternState {
    strings: Option<Arc<FxHashSet<Box<str>>>>,
}

impl InitialInternState {
    /// Record strings already interned by the target interpreter's bootstrap.
    ///
    /// A complete bootstrap profile can be supplied and reused with different Python
    /// targets. Explicit entries are retained for each target. Supplying a string does
    /// not force CPython to intern a constant it would otherwise leave alone.
    pub fn new(strings: impl IntoIterator<Item = impl Into<Box<str>>>) -> Self {
        let strings: FxHashSet<_> = strings.into_iter().map(Into::<Box<str>>::into).collect();
        Self {
            strings: (!strings.is_empty()).then(|| Arc::new(strings)),
        }
    }

    pub(crate) fn contains_for(&self, python_version: PythonVersion, value: &str) -> bool {
        is_preinterned_string_for(python_version, value)
            || self
                .strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
    }
}

#[cfg(test)]
mod tests {
    use super::InitialInternState;
    use crate::PythonVersion;

    #[test]
    fn supplemental_entries_survive_target_selection() {
        // This name is statically interned by 3.14.5 but was removed in 3.15.0rc2.
        let isolated = InitialInternState::default();
        assert!(isolated.contains_for(PythonVersion::Py314, "aggregate_class"));
        assert!(!isolated.contains_for(PythonVersion::Py315, "aggregate_class"));

        let explicit = InitialInternState::new(["aggregate_class"]);
        assert!(explicit.contains_for(PythonVersion::Py314, "aggregate_class"));
        assert!(explicit.contains_for(PythonVersion::Py315, "aggregate_class"));
    }
}
