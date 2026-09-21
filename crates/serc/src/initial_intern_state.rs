use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::PythonVersion;
use crate::target::is_preinterned_string_for;

/// Immutable string intern state present before compiling a Python module.
///
/// CPython can reuse strings interned by its bootstrap or earlier imports when it
/// assembles code objects. This can affect marshal object sharing even when code
/// fields compare equal. The default preserves Serc's isolated CPython startup
/// model. [`Self::new`] supplements that model with existing canonical strings;
/// [`Self::from_complete`] uses exactly the supplied entries.
///
/// Entries describe canonical strings retained throughout compilation. This models
/// a fixed bootstrap, not a long-lived interpreter session in which mortal interned
/// strings may disappear. Compiling an import statement does not execute the import
/// or change this state. Clones share the immutable entries.
#[derive(Clone, Debug, Default)]
pub struct InitialInternState {
    strings: Option<Arc<FxHashSet<Box<str>>>>,
    additional_strings: Option<Arc<FxHashSet<Box<str>>>>,
    complete: bool,
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
            ..Self::default()
        }
    }

    /// Record a complete state without implicit static or startup entries.
    ///
    /// Only supplied strings are considered initially interned for every target.
    /// Compilation still applies the target's normal interning rules.
    pub fn from_complete(strings: impl IntoIterator<Item = impl Into<Box<str>>>) -> Self {
        Self {
            complete: true,
            ..Self::new(strings)
        }
    }

    /// Return an independent state retaining additional canonical strings.
    ///
    /// The original entries stay shared. Extending again copies only earlier additions.
    /// The state's complete or supplemental mode is preserved.
    #[must_use]
    pub fn with_additional_strings(
        &self,
        strings: impl IntoIterator<Item = impl Into<Box<str>>>,
    ) -> Self {
        let mut additional_strings: FxHashSet<_> =
            strings.into_iter().map(Into::<Box<str>>::into).collect();
        if additional_strings.is_empty() {
            return self.clone();
        }
        if let Some(existing) = &self.additional_strings {
            additional_strings.extend(existing.iter().cloned());
        }
        Self {
            strings: self.strings.clone(),
            additional_strings: Some(Arc::new(additional_strings)),
            complete: self.complete,
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn contains_for(&self, python_version: PythonVersion, value: &str) -> bool {
        (!self.complete && is_preinterned_string_for(python_version, value))
            || self
                .strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
            || self
                .additional_strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::InitialInternState;
    use crate::PythonVersion;

    #[test]
    fn additional_entries_share_the_base_without_mutating_siblings() {
        let base = InitialInternState::new(["baseline_entry_918"]);
        let first = base.with_additional_strings(["first_filename_918"]);
        let second = base.with_additional_strings(["second_filename_918"]);
        let repeated = first.with_additional_strings(["second_filename_918"]);

        for state in [&first, &second, &repeated] {
            assert!(Arc::ptr_eq(
                base.strings.as_ref().unwrap(),
                state.strings.as_ref().unwrap()
            ));
            assert!(state.contains_for(PythonVersion::Py314, "baseline_entry_918"));
            assert!(state.contains_for(PythonVersion::Py314, "aggregate_class"));
            assert!(!state.contains_for(PythonVersion::Py315, "aggregate_class"));
        }
        assert!(first.contains_for(PythonVersion::Py314, "first_filename_918"));
        assert!(!first.contains_for(PythonVersion::Py314, "second_filename_918"));
        assert!(second.contains_for(PythonVersion::Py314, "second_filename_918"));
        assert!(!second.contains_for(PythonVersion::Py314, "first_filename_918"));
        assert!(!base.contains_for(PythonVersion::Py314, "first_filename_918"));
        assert!(!base.contains_for(PythonVersion::Py314, "second_filename_918"));
    }

    #[test]
    fn repeated_extensions_preserve_additions() {
        let first = InitialInternState::default()
            .with_additional_strings(["first_filename_918", "shared_component_918"]);
        let second = first.with_additional_strings(["second_filename_918", "shared_component_918"]);

        for value in [
            "first_filename_918",
            "second_filename_918",
            "shared_component_918",
            "aggregate_class",
        ] {
            assert!(second.contains_for(PythonVersion::Py314, value));
        }
        assert_eq!(second.additional_strings.as_ref().unwrap().len(), 3);
        assert!(!first.contains_for(PythonVersion::Py314, "second_filename_918"));
    }

    #[test]
    fn empty_extensions_share_existing_state() {
        let state = InitialInternState::new(["baseline_entry_918"])
            .with_additional_strings(["filename_component_918"]);
        let extended = state.with_additional_strings(std::iter::empty::<&str>());

        assert!(Arc::ptr_eq(
            state.strings.as_ref().unwrap(),
            extended.strings.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            state.additional_strings.as_ref().unwrap(),
            extended.additional_strings.as_ref().unwrap()
        ));
    }

    #[test]
    fn complete_states_retain_exact_entries_through_extensions() {
        let empty = InitialInternState::from_complete(std::iter::empty::<&str>());
        assert!(!empty.contains_for(PythonVersion::Py314, "aggregate_class"));

        let base = InitialInternState::from_complete(["complete_profile_918"]);
        let first = base.with_additional_strings(["first_filename_918"]);
        let repeated = first.with_additional_strings(["second_filename_918"]);
        let unchanged = repeated.with_additional_strings(std::iter::empty::<&str>());
        for state in [&base, &first, &repeated, &unchanged] {
            assert!(state.is_complete());
            assert!(Arc::ptr_eq(
                base.strings.as_ref().unwrap(),
                state.strings.as_ref().unwrap()
            ));
            for version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                assert!(state.contains_for(version, "complete_profile_918"));
                assert!(!state.contains_for(version, "aggregate_class"));
            }
        }
        assert!(unchanged.contains_for(PythonVersion::Py314, "first_filename_918"));
        assert!(unchanged.contains_for(PythonVersion::Py314, "second_filename_918"));
    }

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
