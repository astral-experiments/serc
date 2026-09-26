//! Explicit, input-bound effects of successful compiler warning callbacks.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use ruff_text_size::TextRange;

use crate::code_object::{CodeFilename, CompilationInterningEvent};
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
}

/// Explicit callback outcomes bound to exactly one immutable prepared compilation.
///
/// Obtain this record with [`PreparedCompilation::effects`]. Each ordinal must be recorded
/// once. Clones remain bound to the same input, and cannot be used to finish another plan.
#[derive(Clone, Debug)]
pub struct CompilationWarningEffects {
    identity: Arc<()>,
    effects: Vec<Option<CompilationWarningEffect>>,
}

impl CompilationWarningEffects {
    /// Record one observed invocation outcome. Missing, extra, or duplicate records are errors.
    pub fn record(
        &mut self,
        ordinal: usize,
        effect: CompilationWarningEffect,
    ) -> Result<(), CompileError> {
        let Some(slot) = self.effects.get_mut(ordinal) else {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "warning ordinal {ordinal} is outside this prepared plan",
            )));
        };
        if slot.is_some() {
            return Err(CompileError::InvalidCompilationEffects(format!(
                "warning ordinal {ordinal} was already recorded",
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

/// An immutable warning plan bound to one exact source, filename, target, and option set.
///
/// Preparing discovers native warning invocations without asserting their external effects.
/// It does not provide a serializable code object. Finish with one observed effect for every
/// invocation; omitted entries never mean that a warning was suppressed.
///
/// Like the compiler's bytecode target, the plan models `exec` mode with inherited flags
/// disabled and optimization level zero. The native interpreter must use `bytes_warning=0`;
/// nondefault byte-comparison warnings are not part of this contract. Sources whose exact
/// warning invocation order is not yet modeled cannot produce a prepared plan.
#[derive(Clone, Debug)]
pub struct PreparedCompilation {
    input: CompilationInput,
    filename: CodeFilename,
    options: CompileOptions,
    warnings: Vec<CompilationWarning>,
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
            identity: Arc::new(()),
        };
        let run = Rc::new(WarningRun::discovery());
        prepared.compile(Rc::clone(&run))?;
        run.ensure_complete_plan()?;
        prepared.warnings.clone_from(&run.warnings.borrow());
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

    /// Create an empty outcomes record bound to this exact prepared input.
    pub fn effects(&self) -> CompilationWarningEffects {
        CompilationWarningEffects {
            identity: Arc::clone(&self.identity),
            effects: vec![None; self.warnings.len()],
        }
    }

    /// Finish with exactly one complete effect record for each warning invocation.
    ///
    /// The original input and options are immutable. A fresh lowering verifies that the plan
    /// is consumed exactly, and the result owns all effects needed for later serialization.
    pub fn finish(
        &self,
        effects: &CompilationWarningEffects,
    ) -> Result<CompiledModule, CompileError> {
        if !Arc::ptr_eq(&self.identity, &effects.identity) {
            return Err(CompileError::InvalidCompilationEffects(
                "warning effects belong to a different prepared input".to_string(),
            ));
        }
        let resolved = effects
            .effects
            .iter()
            .enumerate()
            .map(|(ordinal, effect)| {
                effect.clone().ok_or_else(|| {
                    CompileError::InvalidCompilationEffects(format!(
                        "warning ordinal {ordinal} has no observed outcome",
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let run = Rc::new(WarningRun::resolved(resolved));
        let compiled = self.compile(Rc::clone(&run))?;
        run.ensure_complete_plan()?;
        if *run.warnings.borrow() != self.warnings {
            return Err(CompileError::InvalidCompilationEffects(
                "warning invocation plan changed during compilation".to_string(),
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

/// One lowering's effects and discovered descriptor sequence, shared by its code units.
#[derive(Debug, Default)]
pub(crate) struct WarningRun {
    warnings: RefCell<Vec<CompilationWarning>>,
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
                "cannot prepare warning effects: {reason}"
            )));
        }
        Ok(())
    }

    /// Append an invocation and return its retained-table event at the native phase boundary.
    pub(crate) fn record(&self, mut warning: CompilationWarning) -> CompilationInterningEvent {
        let mut warnings = self.warnings.borrow_mut();
        warning.ordinal = warnings.len();
        let effect = self
            .effects
            .as_ref()
            .and_then(|effects| effects.get(warning.ordinal));
        warnings.push(warning);
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
}
