use std::fmt;
use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::PythonVersion;
use crate::target::is_preinterned_string_for;

/// Immutable string intern state present before compiling a Python module.
///
/// CPython can reuse strings interned by its bootstrap or earlier imports when it
/// assembles code objects. This can affect marshal object sharing even when code
/// fields compare equal. The default preserves Serc's isolated CPython startup
/// model, whose exact native build and bootstrap are recorded in the target
/// metadata. [`Self::new`] supplements that model with existing canonical strings;
/// [`Self::from_complete`] uses exactly the supplied entries. [`Self::from_partial`]
/// leaves every unproved entry unknown and rejects byte-relevant uncertainty.
/// [`Self::from_partial_with_absence`] also accepts an immutable absence proof.
///
/// Entries describe canonical strings retained throughout compilation. For byte
/// source they describe the state before source decoding; the standard codec's
/// retained entries are added during decoding. Use
/// [`Self::without_standard_source_codec_effects`] for a custom codec registry,
/// modified standard codec modules or cached codecs, or when the supplied state
/// already includes decoding's effects. This models
/// a fixed bootstrap, not a long-lived interpreter session in which mortal interned
/// strings may disappear. Compiling an import statement does not execute the import
/// or change this state. Clones share the immutable entries.
#[derive(Clone, Debug, Default)]
pub struct InitialInternState {
    strings: Option<Arc<FxHashSet<Box<str>>>>,
    additional_strings: Option<Arc<FxHashSet<Box<str>>>>,
    mode: InitialInternMode,
    skip_standard_source_codec_effects: bool,
}

type AbsencePredicate = dyn Fn(PythonVersion, &str) -> bool + Send + Sync;

#[derive(Clone)]
struct AbsenceProof {
    predicate: Arc<AbsencePredicate>,
    exceptions: Option<Arc<FxHashSet<Box<str>>>>,
}

impl fmt::Debug for AbsenceProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbsenceProof")
            .field("exceptions", &self.exceptions)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
enum InitialInternMode {
    #[default]
    Supplemental,
    Complete,
    Partial {
        absence: Option<AbsenceProof>,
    },
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
            mode: InitialInternMode::Complete,
            ..Self::new(strings)
        }
    }

    /// Record proven retained canonical strings, leaving every other entry unknown.
    ///
    /// No static or startup entries are implicit. Compilation fails if an unknown
    /// entry can affect serialized object identity. Supplied strings must have a
    /// canonical owner retained throughout compilation, independent of its literals.
    pub fn from_partial(strings: impl IntoIterator<Item = impl Into<Box<str>>>) -> Self {
        Self {
            mode: InitialInternMode::Partial { absence: None },
            ..Self::new(strings)
        }
    }

    /// Record retained strings and a proof of absence for other initial entries.
    ///
    /// The predicate returns `true` only when the spelling is proven absent for
    /// the selected target; `false` leaves it unknown. Retained strings, including
    /// later additions, take precedence. No static or startup entries are implicit.
    ///
    /// The predicate must be pure and depend only on the target, spelling, and
    /// immutable captured facts. Its answers must remain identical across calls,
    /// clones, compilation, and later serialization of every compiled module.
    /// It must perform no I/O. Compilation and serialization may call it repeatedly.
    pub fn from_partial_with_absence(
        strings: impl IntoIterator<Item = impl Into<Box<str>>>,
        predicate: impl Fn(PythonVersion, &str) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            mode: InitialInternMode::Partial {
                absence: Some(AbsenceProof {
                    predicate: Arc::new(predicate),
                    exceptions: None,
                }),
            },
            ..Self::new(strings)
        }
    }

    /// Leave these spellings unknown instead of applying the absence predicate.
    ///
    /// Proven retained strings still take precedence. This only masks the optional
    /// predicate; it does not change supplemental or complete states. The baseline
    /// and predicate stay shared; repeated calls copy only earlier exceptions.
    #[must_use]
    pub fn with_absence_exceptions(
        &self,
        strings: impl IntoIterator<Item = impl Into<Box<str>>>,
    ) -> Self {
        let InitialInternMode::Partial {
            absence: Some(proof),
        } = &self.mode
        else {
            return self.clone();
        };
        let mut absence_exceptions: FxHashSet<_> =
            strings.into_iter().map(Into::<Box<str>>::into).collect();
        if absence_exceptions.is_empty() {
            return self.clone();
        }
        if let Some(existing) = &proof.exceptions {
            absence_exceptions.extend(existing.iter().cloned());
        }
        Self {
            mode: InitialInternMode::Partial {
                absence: Some(AbsenceProof {
                    predicate: Arc::clone(&proof.predicate),
                    exceptions: Some(Arc::new(absence_exceptions)),
                }),
            },
            ..self.clone()
        }
    }

    /// Return an independent state retaining additional canonical strings.
    ///
    /// The original entries stay shared. Extending again copies only earlier additions.
    /// The state's complete, partial, or supplemental mode is preserved.
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
            additional_strings: Some(Arc::new(additional_strings)),
            ..self.clone()
        }
    }

    /// Use the supplied state without adding standard source-codec import effects.
    ///
    /// This is intended for callers modeling an interpreter with a custom codec
    /// registry, modified standard codec modules or cached codecs, or a state already
    /// captured after decoding. The caller must account for any strings interned by
    /// its decoder and codec-registry lookup. This has no effect when compiling
    /// already-decoded text.
    #[must_use]
    pub fn without_standard_source_codec_effects(&self) -> Self {
        Self {
            skip_standard_source_codec_effects: true,
            ..self.clone()
        }
    }

    pub(crate) fn models_standard_source_codec_effects(&self) -> bool {
        !self.skip_standard_source_codec_effects
    }

    pub(crate) fn uses_startup_state(&self) -> bool {
        matches!(self.mode, InitialInternMode::Supplemental)
    }

    /// Recognize the unmodified isolated-startup model, without caller-supplied state.
    pub(crate) fn is_isolated_startup(&self) -> bool {
        self.uses_startup_state()
            && self.strings.is_none()
            && self.additional_strings.is_none()
            && self.models_standard_source_codec_effects()
    }

    pub(crate) fn has_unknown_entries(&self) -> bool {
        matches!(self.mode, InitialInternMode::Partial { .. })
    }

    /// Return known presence or absence, or `None` for an unresolved entry.
    pub(crate) fn membership_for(
        &self,
        python_version: PythonVersion,
        value: &str,
    ) -> Option<bool> {
        if (self.uses_startup_state() && is_preinterned_string_for(python_version, value))
            || self
                .strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
            || self
                .additional_strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
        {
            Some(true)
        } else {
            match &self.mode {
                InitialInternMode::Partial { absence } => absence
                    .as_ref()
                    .is_some_and(|proof| {
                        !proof
                            .exceptions
                            .as_ref()
                            .is_some_and(|strings| strings.contains(value))
                            && (proof.predicate)(python_version, value)
                    })
                    .then_some(false),
                InitialInternMode::Supplemental | InitialInternMode::Complete => Some(false),
            }
        }
    }

    pub(crate) fn contains_for(&self, python_version: PythonVersion, value: &str) -> bool {
        self.membership_for(python_version, value)
            .expect("initial intern-state membership must be known")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{AbsenceProof, InitialInternMode, InitialInternState};
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
            assert!(!state.uses_startup_state());
            assert!(!state.has_unknown_entries());
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
    fn partial_states_preserve_unknown_entries_and_shared_positive_facts() {
        let base = InitialInternState::from_partial(["known_owner_973"]);
        let extended = base.with_additional_strings(["path_owner_973"]);
        let unchanged = extended.with_additional_strings(std::iter::empty::<&str>());
        for state in [&base, &extended, &unchanged] {
            assert!(state.has_unknown_entries());
            assert!(!state.uses_startup_state());
            assert!(Arc::ptr_eq(
                base.strings.as_ref().unwrap(),
                state.strings.as_ref().unwrap()
            ));
            assert_eq!(
                state.membership_for(PythonVersion::Py313, "known_owner_973"),
                Some(true)
            );
            assert_eq!(
                state.membership_for(PythonVersion::Py314, "aggregate_class"),
                None
            );
        }
        assert_eq!(
            base.membership_for(PythonVersion::Py313, "path_owner_973"),
            None
        );
        assert_eq!(
            unchanged.membership_for(PythonVersion::Py313, "path_owner_973"),
            Some(true)
        );
    }

    #[test]
    fn absence_predicates_preserve_unknowns_and_target_boundaries() {
        let state =
            InitialInternState::from_partial_with_absence(["proof_retained"], |version, value| {
                version == PythonVersion::Py313 && value.starts_with("proof_")
            });
        assert!(state.has_unknown_entries());
        assert!(!state.uses_startup_state());
        assert_eq!(
            state.membership_for(PythonVersion::Py313, "proof_absent"),
            Some(false)
        );
        assert_eq!(
            state.membership_for(PythonVersion::Py313, "proof_retained"),
            Some(true)
        );
        assert_eq!(
            state.membership_for(PythonVersion::Py313, "outside_domain"),
            None
        );
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert_eq!(state.membership_for(version, "proof_absent"), None);
            assert_eq!(state.membership_for(version, "proof_retained"), Some(true));
        }
    }

    #[test]
    fn absence_exceptions_share_proofs_and_preserve_retained_owners() {
        fn absence_proof(state: &InitialInternState) -> &AbsenceProof {
            match &state.mode {
                InitialInternMode::Partial {
                    absence: Some(proof),
                } => proof,
                _ => panic!("expected a partial state with an absence proof"),
            }
        }

        let base = InitialInternState::from_partial_with_absence(["proof_retained"], |_, value| {
            value.starts_with("proof_")
        });
        let first = base.with_absence_exceptions(["proof_first", "proof_retained"]);
        let sibling = base.with_absence_exceptions(["proof_second"]);
        let repeated = first.with_absence_exceptions(["proof_second", "proof_first"]);
        let extended = repeated.with_additional_strings(["proof_first"]);
        let unchanged = extended.with_absence_exceptions(std::iter::empty::<&str>());
        for state in [&first, &sibling, &repeated, &extended, &unchanged] {
            assert!(Arc::ptr_eq(
                base.strings.as_ref().unwrap(),
                state.strings.as_ref().unwrap()
            ));
            assert!(Arc::ptr_eq(
                &absence_proof(&base).predicate,
                &absence_proof(state).predicate
            ));
            assert_eq!(
                state.membership_for(PythonVersion::Py313, "proof_retained"),
                Some(true)
            );
        }
        assert!(Arc::ptr_eq(
            absence_proof(&extended).exceptions.as_ref().unwrap(),
            absence_proof(&unchanged).exceptions.as_ref().unwrap()
        ));
        assert_eq!(
            base.membership_for(PythonVersion::Py313, "proof_first"),
            Some(false)
        );
        assert_eq!(
            first.membership_for(PythonVersion::Py313, "proof_first"),
            None
        );
        assert_eq!(
            first.membership_for(PythonVersion::Py313, "proof_second"),
            Some(false)
        );
        assert_eq!(
            sibling.membership_for(PythonVersion::Py313, "proof_first"),
            Some(false)
        );
        assert_eq!(
            sibling.membership_for(PythonVersion::Py313, "proof_second"),
            None
        );
        assert_eq!(
            absence_proof(&repeated).exceptions.as_ref().unwrap().len(),
            3
        );
        assert_eq!(
            unchanged.membership_for(PythonVersion::Py313, "proof_first"),
            Some(true)
        );
        assert_eq!(
            unchanged.membership_for(PythonVersion::Py313, "proof_second"),
            None
        );
        assert_eq!(
            unchanged.membership_for(PythonVersion::Py313, "proof_other"),
            Some(false)
        );
        assert_eq!(
            unchanged.membership_for(PythonVersion::Py313, "outside_domain"),
            None
        );
    }

    #[test]
    fn absence_exceptions_only_mask_a_predicate_and_debug_does_not_evaluate_it() {
        for state in [
            InitialInternState::default(),
            InitialInternState::from_complete(std::iter::empty::<&str>()),
            InitialInternState::from_partial(std::iter::empty::<&str>()),
        ] {
            let extended = state.with_absence_exceptions(["proof_absent"]);
            assert_eq!(extended.uses_startup_state(), state.uses_startup_state());
            assert_eq!(extended.has_unknown_entries(), state.has_unknown_entries());
            assert_eq!(
                extended.membership_for(PythonVersion::Py313, "proof_absent"),
                state.membership_for(PythonVersion::Py313, "proof_absent")
            );
        }
        let state =
            InitialInternState::from_partial_with_absence(std::iter::empty::<&str>(), |_, _| {
                panic!("Debug must not evaluate an absence predicate")
            });
        assert!(format!("{state:?}").contains("AbsenceProof"));
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
