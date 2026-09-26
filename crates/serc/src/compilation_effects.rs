//! Explicit, input-bound effects of native parsing and compiler warning callbacks.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use ruff_text_size::TextRange;

use crate::code_object::{CodeFilename, CompilationInterningEvent};
use crate::target::isolated_unicode;
use crate::{CompileError, CompileOptions, CompiledModule, CpythonTarget};

/// The native compilation phase in which a warning is emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompilationWarningPhase {
    /// Tokenization and parsing, before the AST is compiled.
    Parsing,
    /// AST preprocessing, before symbol-table construction.
    Preprocessing,
    /// Bytecode generation, including warnings from discarded code.
    CodeGeneration,
}

/// One invocation of a native compiler warning sink.
///
/// Equal source ranges may occur repeatedly, for example when a finalizer is copied.
/// The ordinal identifies an invocation within its immutable prepared compilation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilationWarning {
    pub(crate) ordinal: usize,
    pub(crate) phase: CompilationWarningPhase,
    pub(crate) kind: &'static str,
    pub(crate) category: &'static str,
    pub(crate) message: String,
    pub(crate) range: TextRange,
}

impl CompilationWarning {
    /// Return this warning's zero-based native invocation ordinal.
    pub fn ordinal(&self) -> usize {
        self.ordinal
    }

    /// Return the native compilation phase.
    pub fn phase(&self) -> CompilationWarningPhase {
        self.phase
    }

    /// Return the stable warning-family identifier.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Return the native Python warning category name.
    pub fn category(&self) -> &'static str {
        self.category
    }

    /// Return the native warning message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the affected UTF-8 byte range in normalized source.
    pub fn range(&self) -> TextRange {
        self.range
    }
}

/// Native Unicode lookups performed while parsing one complete source.
///
/// This is one aggregate effect, not an invocation count: parser speculation can repeat
/// normalization, and named escapes use a separate interpreter-wide lookup cache. Outcomes
/// must preserve the pinned native normalizer and name-lookup semantics. Replacements that
/// change identifiers or decoded strings are outside this compilation contract.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompilationUnicodeParsing {
    pub(crate) identifiers: bool,
    pub(crate) named_escapes: bool,
}

impl CompilationUnicodeParsing {
    /// Whether a raw non-ASCII identifier requires native NFKC normalization.
    pub fn normalizes_identifiers(&self) -> bool {
        self.identifiers
    }

    /// Whether a nonraw Unicode literal requires native named-character lookup.
    pub fn decodes_named_escapes(&self) -> bool {
        self.named_escapes
    }

    pub(crate) fn is_present(self) -> bool {
        self.identifiers || self.named_escapes
    }
}

/// An external effect boundary in native compilation order.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompilationEvent {
    /// All native Unicode parsing lookups, before preprocessing and code generation.
    ///
    /// Plans mixing these lookups with parser warnings are not yet supported, since their
    /// native order cannot be inferred from source offsets.
    UnicodeParsing(CompilationUnicodeParsing),
    /// One compiler warning invocation.
    Warning(CompilationWarning),
}

/// A retained-string outcome for either a parser boundary or a warning invocation.
///
/// Unicode outcomes cover all lookups in the parse and require unchanged native semantics,
/// including normalization results, named-character results, and parser-owned identifiers.
pub type CompilationEffect = CompilationWarningEffect;

/// Input-bound outcomes for every native compilation event.
pub type CompilationEffects = CompilationWarningEffects;

/// Retained canonical strings established by one successful compiler warning invocation.
///
/// This contract includes filtering and formatting, even when the warning is not displayed.
/// Every listed value is canonical in the target's intern table after the invocation and stays
/// retained through serialization. Previously guaranteed canonical owners must remain retained.
/// Releases, callback failures, and references to noninterned objects are outside this contract.
/// Source text alone cannot establish these facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompilationWarningEffect {
    /// The entire invocation is known to make no retained intern-table additions.
    NoRetainedAdditions,
    /// The entire invocation adds exactly these retained canonical strings.
    RetainedStrings(Vec<String>),
    /// These canonical strings are guaranteed retained; all other additions remain unknown.
    ///
    /// This is a partial positive inventory, never proof that an unlisted string is absent.
    /// Finishing still fails if an unlisted string's uncertain identity can affect the bytes.
    PartialRetainedStrings(Vec<String>),
    /// Partial retained additions, with a finite proof that this invocation cannot add
    /// any of `unchanged_strings` to the intern table.
    ///
    /// This preserves an incoming known absence only. It does not establish absence,
    /// recover knowledge lost at an earlier event, or resolve an earlier uncertain owner.
    /// All unlisted additions remain unknown. Retained positives take precedence.
    PartialRetainedStringsWithUnchanged {
        retained_strings: Vec<String>,
        unchanged_strings: Vec<String>,
    },
}

/// Explicit external outcomes bound to exactly one immutable prepared compilation.
///
/// Obtain this record with [`PreparedCompilation::effects`]. Every event must be recorded
/// once. Warning ordinals remain available through [`Self::record`]. Clones remain bound to
/// the same input and cannot be used to finish another plan.
#[derive(Clone, Debug)]
pub struct CompilationWarningEffects {
    identity: Arc<()>,
    effects: Vec<Option<CompilationWarningEffect>>,
    warning_events: Vec<usize>,
}

impl CompilationWarningEffects {
    /// Record one observed invocation outcome. Missing, extra, or duplicate records are errors.
    pub fn record(
        &mut self,
        ordinal: usize,
        effect: CompilationWarningEffect,
    ) -> Result<(), CompileError> {
        let Some(&event) = self.warning_events.get(ordinal) else {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "warning ordinal {ordinal} is outside this prepared plan",
            )));
        };
        self.record_event(event, effect)
    }

    /// Record the outcome at an index in [`PreparedCompilation::events`].
    ///
    /// Warning ordinals remain indices in `warnings()`. Use this method to record both
    /// warnings and parser boundaries in their shared event order.
    pub fn record_event(
        &mut self,
        index: usize,
        effect: CompilationEffect,
    ) -> Result<(), CompileError> {
        let Some(slot) = self.effects.get_mut(index) else {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "event index {index} is outside this prepared plan",
            )));
        };
        if slot.is_some() {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "event index {index} was already recorded",
            )));
        }
        *slot = Some(effect);
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum CompilationInput {
    Text(String),
    Bytes(Vec<u8>),
}

/// An immutable effect plan bound to one exact source, filename, target, and option set.
///
/// Preparing discovers native effect boundaries without asserting their external effects.
/// It does not provide a serializable code object. Finish with one observed effect for every
/// event; omitted entries never mean that a callback had no effects.
///
/// Like the compiler's bytecode target, the plan models `exec` mode with inherited flags
/// disabled and optimization level zero. The native interpreter must use `bytes_warning=0`;
/// nondefault byte-comparison warnings are not part of this contract. Sources whose exact
/// external effect order is not yet modeled cannot produce a prepared plan.
#[derive(Clone, Debug)]
pub struct PreparedCompilation {
    input: CompilationInput,
    filename: CodeFilename,
    options: CompileOptions,
    warnings: Vec<CompilationWarning>,
    events: Vec<CompilationEvent>,
    intern_state_candidates: Vec<String>,
    identity: Arc<()>,
}

impl PreparedCompilation {
    /// Prepare already decoded Python text using an exact filename and option set.
    pub fn text(
        source: &str,
        filename: &str,
        options: &CompileOptions,
    ) -> Result<Self, CompileError> {
        crate::validate_filename(filename)?;
        Self::new(
            CompilationInput::Text(source.to_string()),
            filename.into(),
            options,
        )
    }

    /// Prepare raw Python source bytes without performing filesystem I/O.
    pub fn bytes(
        source: &[u8],
        filename: &str,
        options: &CompileOptions,
    ) -> Result<Self, CompileError> {
        crate::validate_filename(filename)?;
        Self::new(
            CompilationInput::Bytes(source.to_vec()),
            filename.into(),
            options,
        )
    }

    /// Prepare raw bytes while preserving Python's operating-system filename representation.
    pub fn bytes_with_path(
        source: &[u8],
        filename: &Path,
        options: &CompileOptions,
    ) -> Result<Self, CompileError> {
        if filename.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(CompileError::InvalidFilename);
        }
        Self::new(
            CompilationInput::Bytes(source.to_vec()),
            CodeFilename::from_path(filename),
            options,
        )
    }

    fn new(
        input: CompilationInput,
        filename: CodeFilename,
        options: &CompileOptions,
    ) -> Result<Self, CompileError> {
        let mut prepared = Self {
            input,
            filename,
            options: options.clone(),
            warnings: Vec::new(),
            events: Vec::new(),
            intern_state_candidates: Vec::new(),
            identity: Arc::new(()),
        };
        let run = Rc::new(WarningRun::discovery());
        let compiled = prepared.compile(Rc::clone(&run))?;
        run.ensure_complete_plan()?;
        prepared.warnings.clone_from(&run.warnings.borrow());
        prepared.events.clone_from(&run.events.borrow());
        prepared.intern_state_candidates =
            crate::marshal::intern_state_candidates(&compiled.code, compiled.python_version);
        Ok(prepared)
    }

    /// Return the exact target to which this plan is bound.
    pub fn target(&self) -> CpythonTarget {
        self.options.python_version.target()
    }

    /// Return native warning invocations in their actual compilation order.
    pub fn warnings(&self) -> &[CompilationWarning] {
        &self.warnings
    }

    /// Return external effect boundaries in native compilation order.
    pub fn events(&self) -> &[CompilationEvent] {
        &self.events
    }

    /// Return sorted candidate spellings whose intern identity can affect this plan.
    ///
    /// These come from assembly provenance, including discarded scopes, and sensitive
    /// frozen-set members. They are not membership or ownership facts. A caller may use
    /// them to restrict an independently justified external-effect proof to this input;
    /// omitted or unproven spellings retain their ordinary uncertainty.
    pub fn intern_state_candidates(&self) -> &[String] {
        &self.intern_state_candidates
    }

    /// Create an empty outcomes record bound to this exact prepared input.
    pub fn effects(&self) -> CompilationWarningEffects {
        CompilationWarningEffects {
            identity: Arc::clone(&self.identity),
            effects: vec![None; self.events.len()],
            warning_events: self
                .events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    matches!(event, CompilationEvent::Warning(_)).then_some(index)
                })
                .collect(),
        }
    }

    /// Model this input's Unicode lookup under the isolated CPython target contract.
    ///
    /// This explicitly selects the target's pristine native normalizer, name capsule,
    /// loader, and retained-owner semantics. It does not inspect or qualify an installed
    /// interpreter. The generated source facts establish partial retained additions and
    /// preserve incoming absence only for a finite, independently excluded set of names.
    /// Other uncertain membership still fails when it can affect serialized bytes.
    ///
    /// The plan must use the unmodified default startup state and standard source codecs,
    /// and contain exactly one Unicode parsing event. Warning callbacks, custom caller
    /// states, and unsupported parser interleavings require separately supplied effects.
    /// Ordinary compilation and [`Self::effects`] do not select this model implicitly.
    pub fn isolated_unicode_effects(&self) -> Result<CompilationEffects, CompileError> {
        if !self.options.initial_intern_state.is_isolated_startup()
            || !matches!(
                self.events.as_slice(),
                [CompilationEvent::UnicodeParsing(_)]
            )
        {
            return Err(CompileError::InvalidCompilationEffects(
                "isolated Unicode effects require the default startup state, standard source codecs, and one Unicode event without warnings".to_string(),
            ));
        }

        let retained = match self.options.python_version {
            crate::PythonVersion::Py312 => isolated_unicode::RETAINED312,
            crate::PythonVersion::Py313 => isolated_unicode::RETAINED313,
            crate::PythonVersion::Py314 => isolated_unicode::RETAINED314,
            crate::PythonVersion::Py315 => isolated_unicode::RETAINED315,
        };
        let mut effects = self.effects();
        effects.record_event(
            0,
            CompilationEffect::PartialRetainedStringsWithUnchanged {
                retained_strings: retained.iter().map(|value| (*value).to_string()).collect(),
                unchanged_strings: self
                    .intern_state_candidates
                    .iter()
                    .filter(|value| {
                        isolated_unicode_preserves_absence(self.options.python_version, value)
                    })
                    .cloned()
                    .collect(),
            },
        )?;
        Ok(effects)
    }

    /// Finish with exactly one effect record for each native event.
    ///
    /// The original input and options are immutable. A fresh lowering verifies that the plan
    /// is consumed exactly, and the result owns all effects needed for later serialization.
    pub fn finish(
        &self,
        effects: &CompilationWarningEffects,
    ) -> Result<CompiledModule, CompileError> {
        if !Arc::ptr_eq(&self.identity, &effects.identity) {
            return Err(CompileError::InvalidCompilationEffects(
                "compilation effects belong to a different prepared input".to_string(),
            ));
        }
        let resolved = effects
            .effects
            .iter()
            .enumerate()
            .map(|(ordinal, effect)| {
                effect.clone().ok_or_else(|| {
                    CompileError::InvalidCompilationEffects(format!(
                        "event index {ordinal} has no observed outcome",
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let run = Rc::new(WarningRun::resolved(resolved));
        let compiled = self.compile(Rc::clone(&run))?;
        run.ensure_complete_plan()?;
        if *run.events.borrow() != self.events {
            return Err(CompileError::InvalidCompilationEffects(
                "native effect plan changed during compilation".to_string(),
            ));
        }
        Ok(compiled)
    }

    fn compile(&self, run: Rc<WarningRun>) -> Result<CompiledModule, CompileError> {
        match &self.input {
            CompilationInput::Text(source) => {
                crate::compile_text_source(source, self.filename.clone(), &self.options, Some(run))
            }
            CompilationInput::Bytes(source) => crate::compile_byte_source_with_warnings(
                source,
                self.filename.clone(),
                &self.options,
                Some(run),
            ),
        }
    }
}

/// Exclude only names outside the source-derived native loader and Unicode namespaces.
fn isolated_unicode_preserves_absence(version: crate::PythonVersion, value: &str) -> bool {
    let target_mask = match version {
        crate::PythonVersion::Py312 => return false,
        crate::PythonVersion::Py313 => isolated_unicode::POSSIBLE_D313_MASK,
        crate::PythonVersion::Py314 => isolated_unicode::POSSIBLE_D314_MASK,
        crate::PythonVersion::Py315 => isolated_unicode::POSSIBLE_D315_MASK,
    };
    let identifier = |part: &str| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    };
    let Some((first, rest)) = value.split_once('.') else {
        return false;
    };
    identifier(first)
        && rest
            .split('.')
            .all(|part| part == "<locals>" || identifier(part))
        && match isolated_unicode::POSSIBLE_D.binary_search_by_key(&value, |(name, _)| *name) {
            Ok(index) => isolated_unicode::POSSIBLE_D[index].1 & target_mask == 0,
            Err(_) => true,
        }
        && !isolated_unicode::EXCLUDED_PREFIXES
            .iter()
            .any(|prefix| value.starts_with(prefix))
}

/// One lowering's effects and discovered descriptor sequence, shared by its code units.
#[derive(Debug, Default)]
pub(crate) struct WarningRun {
    warnings: RefCell<Vec<CompilationWarning>>,
    events: RefCell<Vec<CompilationEvent>>,
    effects: Option<Vec<CompilationWarningEffect>>,
    discovery: bool,
    incomplete_plan: RefCell<Option<String>>,
}

impl WarningRun {
    fn discovery() -> Self {
        Self {
            discovery: true,
            ..Self::default()
        }
    }

    fn resolved(effects: Vec<CompilationWarningEffect>) -> Self {
        Self {
            effects: Some(effects),
            ..Self::default()
        }
    }

    pub(crate) fn is_discovery(&self) -> bool {
        self.discovery
    }

    /// Preserve uncertainty when the native warning invocation order is not modeled.
    pub(crate) fn invalidate_plan(&self, reason: impl Into<String>) {
        self.incomplete_plan
            .borrow_mut()
            .get_or_insert_with(|| reason.into());
    }

    fn ensure_complete_plan(&self) -> Result<(), CompileError> {
        if let Some(reason) = &*self.incomplete_plan.borrow() {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "cannot prepare compilation effects: {reason}"
            )));
        }
        Ok(())
    }

    /// Append an invocation and return its retained-table event at the native phase boundary.
    pub(crate) fn record(&self, mut warning: CompilationWarning) -> CompilationInterningEvent {
        let mut warnings = self.warnings.borrow_mut();
        warning.ordinal = warnings.len();
        warnings.push(warning.clone());
        self.record_event(CompilationEvent::Warning(warning))
    }

    /// Record the complete parser lookup aggregate without changing parser-owned origins.
    pub(crate) fn record_unicode(
        &self,
        unicode: CompilationUnicodeParsing,
    ) -> CompilationInterningEvent {
        self.record_event(CompilationEvent::UnicodeParsing(unicode))
    }

    fn record_event(&self, event: CompilationEvent) -> CompilationInterningEvent {
        let mut events = self.events.borrow_mut();
        let effect = self
            .effects
            .as_ref()
            .and_then(|effects| effects.get(events.len()));
        events.push(event);
        match effect {
            Some(CompilationWarningEffect::NoRetainedAdditions) => {
                CompilationInterningEvent::ExternalRetain(Vec::new())
            }
            Some(CompilationWarningEffect::RetainedStrings(values)) => {
                CompilationInterningEvent::ExternalRetain(values.clone())
            }
            Some(CompilationWarningEffect::PartialRetainedStrings(values)) => {
                CompilationInterningEvent::PartialExternalRetain(values.clone())
            }
            Some(CompilationWarningEffect::PartialRetainedStringsWithUnchanged {
                retained_strings,
                unchanged_strings,
            }) => CompilationInterningEvent::PartialExternalRetainWithUnchanged {
                retained_strings: retained_strings.clone(),
                unchanged_strings: unchanged_strings.clone(),
            },
            None => CompilationInterningEvent::UnknownExternalAdditions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InitialInternState, PythonVersion, compile_with_options};

    const FIRST: &str =
        "def first(value):\n    return value in {'warning_marker_796', 'other_marker_796'}, 1\n";
    const SECOND: &str = "def second(value):\n    \"second docstring\"\n    return value in {'warning_marker_796', 'other_marker_796'}, 2\n";
    const WARNING: &str = "if 1 is 1:\n    pass\n";

    fn effects(
        plan: &PreparedCompilation,
        effect: CompilationWarningEffect,
    ) -> CompilationWarningEffects {
        let mut effects = plan.effects();
        for (warning, effect) in plan
            .warnings()
            .iter()
            .zip(std::iter::repeat_n(effect, plan.warnings().len()))
        {
            effects.record(warning.ordinal(), effect).unwrap();
        }
        effects
    }

    fn options(version: PythonVersion) -> CompileOptions {
        CompileOptions {
            python_version: version,
            initial_intern_state: InitialInternState::from_complete(std::iter::empty::<&str>()),
            ..CompileOptions::default()
        }
    }

    #[test]
    fn codegen_effects_observe_first_constant_interning() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for (source, affects_identity) in [
                (format!("{WARNING}{FIRST}{SECOND}"), true),
                (format!("{FIRST}{WARNING}{SECOND}"), false),
                (format!("{FIRST}{SECOND}{WARNING}"), false),
            ] {
                let plan =
                    PreparedCompilation::text(&source, "warning_effects.py", &options(version))
                        .unwrap();
                assert_eq!(plan.warnings().len(), 1);
                assert_eq!(
                    plan.warnings()[0].phase(),
                    CompilationWarningPhase::CodeGeneration
                );
                let quiet = plan
                    .finish(&effects(
                        &plan,
                        CompilationWarningEffect::NoRetainedAdditions,
                    ))
                    .unwrap()
                    .marshal();
                let retained = plan
                    .finish(&effects(
                        &plan,
                        CompilationWarningEffect::RetainedStrings(vec![
                            "warning_marker_796".to_string(),
                        ]),
                    ))
                    .unwrap()
                    .marshal();
                assert_eq!(quiet != retained, affects_identity, "{version:?}: {source}");
                let default =
                    compile_with_options(&source, "warning_effects.py", &options(version));
                assert_eq!(
                    default.is_err(),
                    affects_identity,
                    "{version:?}: {default:?}"
                );
            }
        }
    }

    #[test]
    fn partial_effects_resolve_only_proved_canonical_owners() {
        let strings = vec![
            "warning_marker_796".to_string(),
            "other_marker_796".to_string(),
        ];
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let source = format!("{WARNING}{FIRST}{SECOND}");
            let plan = PreparedCompilation::text(&source, "partial_warning.py", &options(version))
                .unwrap();
            let complete = effects(
                &plan,
                CompilationWarningEffect::RetainedStrings(strings.clone()),
            );
            let partial = effects(
                &plan,
                CompilationWarningEffect::PartialRetainedStrings(strings.clone()),
            );
            assert_eq!(
                plan.finish(&partial).unwrap().marshal(),
                plan.finish(&complete).unwrap().marshal()
            );

            // One guaranteed replacement already fixes each set's recreation. Unknown
            // membership of the other member cannot change this module's bytes.
            let sufficient = effects(
                &plan,
                CompilationWarningEffect::PartialRetainedStrings(vec![strings[0].clone()]),
            );
            assert_eq!(
                plan.finish(&sufficient).unwrap().marshal(),
                plan.finish(&complete).unwrap().marshal()
            );
            let missing = effects(
                &plan,
                CompilationWarningEffect::PartialRetainedStrings(Vec::new()),
            );
            assert!(matches!(
                plan.finish(&missing),
                Err(CompileError::UnresolvedInternState { .. })
            ));

            let source = format!("{FIRST}{WARNING}{SECOND}");
            let plan = PreparedCompilation::text(&source, "partial_warning.py", &options(version))
                .unwrap();
            let partial = effects(
                &plan,
                CompilationWarningEffect::PartialRetainedStrings(Vec::new()),
            );
            let empty = effects(&plan, CompilationWarningEffect::NoRetainedAdditions);
            assert_eq!(
                plan.finish(&partial).unwrap().marshal(),
                plan.finish(&empty).unwrap().marshal()
            );
        }
    }

    #[test]
    fn partial_warning_effects_invalidate_initial_absence_without_losing_retained_owners() {
        const QUALIFIED: &str = "WarningScope.Inner";
        const SOURCE: &str = "if 1 is 1: pass\nclass WarningScope:\n    class Inner: pass\n";
        for version in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for initially_retained in [false, true] {
                let initial = crate::InitialInternState::from_partial_with_absence(
                    initially_retained.then_some(QUALIFIED),
                    |_, value| value == QUALIFIED,
                );
                let options = CompileOptions {
                    initial_intern_state: initial,
                    ..options(version)
                };
                let plan =
                    PreparedCompilation::text(SOURCE, "warning_absence.py", &options).unwrap();
                let quiet = plan
                    .finish(&effects(
                        &plan,
                        CompilationWarningEffect::NoRetainedAdditions,
                    ))
                    .unwrap()
                    .marshal();
                let partial = plan.finish(&effects(
                    &plan,
                    CompilationWarningEffect::PartialRetainedStrings(Vec::new()),
                ));
                assert!(
                    plan.intern_state_candidates()
                        .iter()
                        .any(|value| value == QUALIFIED)
                );
                let preserved = plan
                    .finish(&effects(
                        &plan,
                        CompilationEffect::PartialRetainedStringsWithUnchanged {
                            retained_strings: Vec::new(),
                            unchanged_strings: vec![QUALIFIED.to_string()],
                        },
                    ))
                    .unwrap();
                assert_eq!(preserved.marshal(), quiet);
                if initially_retained {
                    assert_eq!(partial.unwrap().marshal(), quiet);
                } else {
                    assert!(
                        matches!(partial, Err(CompileError::UnresolvedInternState { value }) if value == QUALIFIED)
                    );
                    let supplied = vec![QUALIFIED.to_string()];
                    let partial = plan
                        .finish(&effects(
                            &plan,
                            CompilationWarningEffect::PartialRetainedStrings(supplied.clone()),
                        ))
                        .unwrap();
                    let complete = plan
                        .finish(&effects(
                            &plan,
                            CompilationWarningEffect::RetainedStrings(supplied),
                        ))
                        .unwrap();
                    assert_eq!(partial.marshal(), complete.marshal());
                }
            }
        }
    }

    #[test]
    fn parser_effect_precedes_all_assemblies_even_at_end_of_source() {
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let source = format!("{FIRST}{SECOND}text = '\\z'\n");
            let plan = PreparedCompilation::text(&source, "warning_effects.py", &options(version))
                .unwrap();
            assert_eq!(plan.warnings().len(), 1);
            assert_eq!(plan.warnings()[0].phase(), CompilationWarningPhase::Parsing);
            assert!(matches!(
                compile_with_options(&source, "warning_effects.py", &options(version)),
                Err(CompileError::UnresolvedInternState { .. })
            ));
            let quiet = plan
                .finish(&effects(
                    &plan,
                    CompilationWarningEffect::NoRetainedAdditions,
                ))
                .unwrap()
                .marshal();
            let retained = plan
                .finish(&effects(
                    &plan,
                    CompilationWarningEffect::RetainedStrings(vec![
                        "warning_marker_796".to_string(),
                    ]),
                ))
                .unwrap()
                .marshal();
            assert_ne!(quiet, retained, "{version:?}");
        }
    }

    #[test]
    fn effects_are_explicit_and_owned_by_completed_module() {
        let source = format!("{WARNING}{FIRST}{SECOND}");
        let plan = PreparedCompilation::bytes(
            source.as_bytes(),
            "warning_effects.py",
            &options(PythonVersion::Py314),
        )
        .unwrap();
        assert!(matches!(
            plan.finish(&plan.effects()),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        let mut rejected = plan.effects();
        assert!(
            rejected
                .record(
                    plan.warnings().len(),
                    CompilationWarningEffect::NoRetainedAdditions
                )
                .is_err()
        );
        rejected
            .record(0, CompilationWarningEffect::NoRetainedAdditions)
            .unwrap();
        assert!(
            rejected
                .record(0, CompilationWarningEffect::NoRetainedAdditions)
                .is_err()
        );
        let other = PreparedCompilation::text(
            &source,
            "different_filename.py",
            &options(PythonVersion::Py314),
        )
        .unwrap();
        assert!(matches!(
            other.finish(&rejected),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        let effects = effects(
            &plan,
            CompilationWarningEffect::RetainedStrings(vec!["warning_marker_796".to_string()]),
        );
        let compiled = plan.finish(&effects).unwrap();
        let expected = compiled.marshal();
        drop(effects);
        drop(plan);
        assert_eq!(compiled.marshal(), expected);
        assert_eq!(compiled.marshal(), expected);
        assert_eq!(
            compiled.to_timestamp_pyc(0, 0),
            compiled.to_timestamp_pyc(0, 0)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    fn prepared_effects_preserve_selected_native_profile() {
        let source = "if 1 is 1: pass\nvalue = (1e309 - 1e309 + 1j) / (1 + 2j)\n";
        let mut outputs = Vec::new();
        for native_build_profile in [
            crate::NativeBuildProfile::Cpython3145ActionsGcc,
            crate::NativeBuildProfile::Cpython3145UvClang,
        ] {
            let options = CompileOptions {
                native_build_profile,
                ..CompileOptions::default()
            };
            let expected = compile_with_options(source, "profile_warnings.py", &options)
                .unwrap()
                .marshal();
            let plan = PreparedCompilation::text(source, "profile_warnings.py", &options).unwrap();
            let effects = effects(&plan, CompilationWarningEffect::NoRetainedAdditions);
            let actual = plan.finish(&effects).unwrap().marshal();
            assert_eq!(actual, expected);
            outputs.push(actual);
        }
        assert_ne!(outputs[0], outputs[1]);
    }

    #[test]
    fn warnings_without_identity_dependencies_remain_compilable() {
        assert!(crate::compile("if 1 is 1:\n    pass\n", "warning_effects.py").is_ok());
        let source = "class Outer:\n    class Inner: pass\n";
        let options = options(PythonVersion::Py314);
        let plan = PreparedCompilation::text(source, "warning_effects.py", &options).unwrap();
        assert!(plan.warnings().is_empty());
        assert_eq!(
            plan.finish(&plan.effects()).unwrap().marshal(),
            compile_with_options(source, "warning_effects.py", &options)
                .unwrap()
                .marshal()
        );
    }

    #[test]
    fn unicode_parsing_effect_is_explicit_and_precedes_every_assembly() {
        let first =
            "def first(value):\n    return value in {'normalize', 'unicode_marker_789'}, 1\n";
        let second = "def second(value):\n    'second docstring'\n    return value in {'normalize', 'unicode_marker_789'}, 2\n";
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for trigger in [
                "слово = 1\n",
                "\u{212a} = 1\n",
                "text = '\\N{LATIN CAPITAL LETTER A}'\n",
            ] {
                // Even a final parser operation precedes all code-object construction.
                let source = format!("{first}{second}{trigger}");
                let plan =
                    PreparedCompilation::text(&source, "unicode_effects.py", &options(version))
                        .unwrap();
                assert!(matches!(
                    plan.events(),
                    [CompilationEvent::UnicodeParsing(_)]
                ));
                assert!(plan.warnings().is_empty());
                assert!(matches!(
                    plan.finish(&plan.effects()),
                    Err(CompileError::InvalidCompilationEffects(_))
                ));
                assert!(matches!(
                    compile_with_options(&source, "unicode_effects.py", &options(version)),
                    Err(CompileError::UnresolvedInternState { .. })
                ));
                let mut absent = plan.effects();
                absent
                    .record_event(0, CompilationEffect::NoRetainedAdditions)
                    .unwrap();
                let absent = plan.finish(&absent).unwrap().marshal();
                let mut retained = plan.effects();
                retained
                    .record_event(
                        0,
                        CompilationEffect::PartialRetainedStrings(vec!["normalize".to_string()]),
                    )
                    .unwrap();
                let retained = plan.finish(&retained).unwrap().marshal();
                assert_ne!(absent, retained, "{version:?}: {source}");
            }
        }
    }

    #[test]
    fn unicode_event_and_warning_ordinals_share_one_bound_record() {
        let source = format!("\u{212a} = 1\n{WARNING}{FIRST}{SECOND}");
        let plan = PreparedCompilation::text(
            &source,
            "unicode_warning.py",
            &options(PythonVersion::Py314),
        )
        .unwrap();
        assert!(matches!(
            plan.events(),
            [
                CompilationEvent::UnicodeParsing(_),
                CompilationEvent::Warning(_)
            ]
        ));
        assert_eq!(plan.warnings()[0].ordinal(), 0);
        let mut effects = plan.effects();
        effects
            .record(0, CompilationWarningEffect::NoRetainedAdditions)
            .unwrap();
        assert!(matches!(
            plan.finish(&effects),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        assert!(
            effects
                .record_event(1, CompilationEffect::NoRetainedAdditions)
                .is_err()
        );
        effects
            .record_event(
                0,
                CompilationEffect::RetainedStrings(vec!["warning_marker_796".to_string()]),
            )
            .unwrap();
        let bytes = plan.finish(&effects).unwrap().marshal();
        let other =
            PreparedCompilation::text(&source, "other.py", &options(PythonVersion::Py314)).unwrap();
        assert!(matches!(
            other.finish(&effects),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        assert_eq!(bytes, plan.finish(&effects).unwrap().marshal());
    }

    #[test]
    fn unicode_partial_effect_does_not_preserve_unproved_absence() {
        let source = format!("\u{212a} = 1\n{FIRST}{SECOND}");
        let options = CompileOptions {
            initial_intern_state: InitialInternState::from_partial_with_absence(
                std::iter::empty::<&str>(),
                |_, _| true,
            ),
            ..CompileOptions::default()
        };
        let plan = PreparedCompilation::text(&source, "unicode_unknown.py", &options).unwrap();
        let mut effects = plan.effects();
        effects
            .record_event(
                0,
                CompilationEffect::PartialRetainedStrings(vec!["normalize".to_string()]),
            )
            .unwrap();
        assert!(matches!(
            plan.finish(&effects),
            Err(CompileError::UnresolvedInternState { .. })
        ));
    }

    #[test]
    fn unicode_parser_warning_interleaving_requires_an_exact_plan() {
        for source in [
            "\u{212a} = 1\ntext = '\\z'\n",
            "text = '\\z'\n\u{212a} = 1\n",
            "text = '\\z\\N{LATIN CAPITAL LETTER A}'\n",
            "text = f'\\N{LATIN CAPITAL LETTER A}{value}\\z'\n",
        ] {
            assert!(
                matches!(
                    PreparedCompilation::text(
                        source,
                        "unicode_order.py",
                        &CompileOptions::default()
                    ),
                    Err(CompileError::InvalidCompilationEffects(_))
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn isolated_unicode_effects_are_explicit_and_bound_to_the_default_plan() {
        let source = "class UnitBase:\n    class EquivalentUnitsList:\n        pass\nmicro = '\\N{MICRO SIGN}'\n";
        let options = CompileOptions::default();
        assert!(matches!(
            compile_with_options(source, "isolated.py", &options),
            Err(CompileError::UnresolvedInternState { .. })
        ));
        let plan = PreparedCompilation::text(source, "isolated.py", &options).unwrap();
        let effects = plan.isolated_unicode_effects().unwrap();
        plan.finish(&effects).unwrap();

        let other = PreparedCompilation::text(source, "different.py", &options).unwrap();
        assert!(matches!(
            other.finish(&effects),
            Err(CompileError::InvalidCompilationEffects(_))
        ));
        for state in [
            InitialInternState::new(["caller_owned"]),
            InitialInternState::from_complete(std::iter::empty::<&str>()),
            InitialInternState::from_partial(std::iter::empty::<&str>()),
            InitialInternState::default().without_standard_source_codec_effects(),
        ] {
            let options = CompileOptions {
                initial_intern_state: state,
                ..CompileOptions::default()
            };
            let custom = PreparedCompilation::text(source, "custom.py", &options).unwrap();
            assert!(matches!(
                custom.isolated_unicode_effects(),
                Err(CompileError::InvalidCompilationEffects(_))
            ));
        }
        for source in [
            "value = 1\n".to_string(),
            format!("{source}if 1 is 1:\n    pass\n"),
        ] {
            let other = PreparedCompilation::text(&source, "unsupported.py", &options).unwrap();
            assert!(matches!(
                other.isolated_unicode_effects(),
                Err(CompileError::InvalidCompilationEffects(_))
            ));
        }
    }

    #[test]
    fn isolated_unicode_model_preserves_no_unproved_absence() {
        let source =
            "class UCD:\n    class CustomOwner:\n        pass\nmicro = '\\N{MICRO SIGN}'\n";
        let plan =
            PreparedCompilation::text(source, "unknown.py", &CompileOptions::default()).unwrap();
        assert!(matches!(
            plan.finish(&plan.isolated_unicode_effects().unwrap()),
            Err(CompileError::UnresolvedInternState { .. })
        ));
    }
}
