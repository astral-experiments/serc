use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::target::is_preinterned_string;

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
    /// A complete bootstrap profile can be supplied. Entries already covered by the
    /// isolated target are omitted from storage. Supplying a string does not force
    /// CPython to intern a constant it would otherwise leave alone.
    pub fn new(strings: impl IntoIterator<Item = impl Into<Box<str>>>) -> Self {
        let strings: FxHashSet<_> = strings
            .into_iter()
            .map(Into::<Box<str>>::into)
            .filter(|value| !is_preinterned_string(value))
            .collect();
        Self {
            strings: (!strings.is_empty()).then(|| Arc::new(strings)),
        }
    }

    pub(crate) fn contains(&self, value: &str) -> bool {
        is_preinterned_string(value)
            || self
                .strings
                .as_ref()
                .is_some_and(|strings| strings.contains(value))
    }
}
