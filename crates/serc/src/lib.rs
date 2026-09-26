//! Compile Python source to CPython bytecode without starting a Python interpreter.
//!
//! Select Python 3.12, 3.13, 3.14, or 3.15 with [`CompileOptions`]; Python 3.14 is the default.
//! [`PythonVersion::target`] identifies each pinned CPython release. Bytecode, code objects,
//! marshal, and `.pyc` formats are implementation details tied to those exact releases.

mod assembler;
mod code_object;
mod compiler;
mod config;
mod delimiter_nesting;
mod indentation;
mod initial_intern_state;
mod marshal;
mod source;
mod stack;
mod target;
mod yield_syntax;

use std::error::Error;
use std::fmt;
use std::path::Path;

use code_object::{CodeFilename, CodeObject};
use compiler::Compiler;
pub use config::{CompileOptions, NativeBuildProfile, ParsePythonVersionError, PythonVersion};
pub use initial_intern_state::InitialInternState;
use ruff_text_size::{TextRange, TextSize};
use siphasher::sip::SipHasher13;
pub use source::{SourceDecodeError, decode_source};

/// The exact CPython implementation targeted by this compiler backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpythonTarget {
    /// The value of `sys.implementation.name`.
    pub implementation: &'static str,
    /// The value of `sys.version_info[:3]`.
    pub version: (u8, u8, u8),
    /// The value of `importlib.util.MAGIC_NUMBER`.
    pub magic_number: [u8; 4],
    /// The upstream CPython release tag.
    pub tag: &'static str,
    /// The upstream CPython source commit.
    pub commit: &'static str,
}

impl CpythonTarget {
    /// Return the dotted form of [`Self::version`].
    pub fn version_string(self) -> String {
        let (major, minor, micro) = self.version;
        format!("{major}.{minor}.{micro}")
    }

    /// Return the lowercase hexadecimal form of [`Self::magic_number`].
    pub fn magic_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";

        let mut output = String::with_capacity(self.magic_number.len() * 2);
        for byte in self.magic_number {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }
}

/// The default CPython target. See [`PythonVersion::target`] for other supported versions.
pub const CPYTHON_TARGET: CpythonTarget = CpythonTarget {
    implementation: target::TARGET_IMPLEMENTATION,
    version: target::TARGET_VERSION,
    magic_number: target::TARGET_MAGIC_NUMBER,
    tag: target::TARGET_TAG,
    commit: target::TARGET_COMMIT,
};

/// A compiled module code object and its selected CPython target.
#[derive(Clone, Debug)]
pub struct CompiledModule {
    code: CodeObject,
    python_version: PythonVersion,
    initial_intern_state: InitialInternState,
}

impl CompiledModule {
    /// Return the Python version used to compile and serialize this module.
    pub fn python_version(&self) -> PythonVersion {
        self.python_version
    }

    /// Return the exact CPython target for this module.
    pub fn target(&self) -> CpythonTarget {
        self.python_version.target()
    }

    /// Return the module's raw CPython instruction stream.
    pub fn bytecode(&self) -> &[u8] {
        &self.code.bytecode
    }

    /// Marshal the module code object using its selected CPython target's format.
    pub fn marshal(&self) -> Vec<u8> {
        marshal::encode_code_with_options(
            &self.code,
            self.python_version,
            &self.initial_intern_state,
        )
    }

    /// Build a timestamp-invalidated `.pyc` file for the selected CPython target.
    ///
    /// `source_mtime` is the source file's modification time in whole seconds since the
    /// Unix epoch. `source_size` is the source file's byte length, truncated to 32 bits as
    /// required by CPython's cache format. Root-object references follow `py_compile`,
    /// which can differ from serializing a retained code object with [`Self::marshal`].
    pub fn to_timestamp_pyc(&self, source_mtime: u32, source_size: u32) -> Vec<u8> {
        let mut metadata = [0; 8];
        metadata[..4].copy_from_slice(&source_mtime.to_le_bytes());
        metadata[4..].copy_from_slice(&source_size.to_le_bytes());
        self.to_pyc(0, metadata)
    }

    /// Build a hash-invalidated `.pyc` file for the selected CPython target.
    ///
    /// `source` must be the original source bytes used to compile the module, before
    /// decoding or newline normalization. If `checked` is true, CPython normally
    /// verifies the hash against the source on import. Otherwise, it normally trusts
    /// an external system to keep the cache current. Python's `--check-hash-based-pycs`
    /// option can override this behavior.
    ///
    /// Root-object references follow `py_compile`, as in [`Self::to_timestamp_pyc`].
    pub fn to_hash_pyc(&self, source: &[u8], checked: bool) -> Vec<u8> {
        // CPython's _imp.source_hash uses SipHash-1-3 keyed by the target's magic
        // number and serializes the resulting hash in little-endian order.
        let key = u64::from(u32::from_le_bytes(self.target().magic_number));
        let hash = SipHasher13::new_with_keys(key, 0).hash(source);
        self.to_pyc(1 | (u32::from(checked) << 1), hash.to_le_bytes())
    }

    fn to_pyc(&self, flags: u32, metadata: [u8; 8]) -> Vec<u8> {
        let mut header = [0; 16];
        header[..4].copy_from_slice(&self.target().magic_number);
        header[4..8].copy_from_slice(&flags.to_le_bytes());
        header[8..].copy_from_slice(&metadata);
        marshal::encode_pyc_code_with_options(
            &self.code,
            self.python_version,
            &self.initial_intern_state,
            &header,
        )
    }
}

/// A filename validation, source decoding, parsing, or bytecode generation error.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompileError {
    /// The filename contains a null character.
    InvalidFilename,
    /// The Python source bytes could not be decoded.
    Decode(SourceDecodeError),
    /// Ruff rejected the Python source.
    Parse(ruff_python_parser::ParseError),
    /// A pattern violates Python's semantic restrictions.
    InvalidPattern {
        /// The restriction violated by the pattern.
        message: String,
        /// The byte range of the invalid pattern in the normalized source.
        range: TextRange,
    },
    /// The parsed source violates a syntax restriction of the targeted Python version.
    InvalidSyntax {
        /// The syntax restriction violated by the source.
        message: String,
        /// The byte range of the invalid syntax in the normalized source.
        range: TextRange,
    },
    /// CPython could not encode a docstring while preparing the parsed source.
    InvalidDocstring {
        /// The UTF-8 encoding error produced for the docstring.
        message: String,
        /// The byte range of the docstring literal in the normalized source.
        range: TextRange,
    },
    /// The source uses valid Python syntax that this backend does not support yet.
    Unsupported(String),
    /// Initial intern-state uncertainty can affect serialized object identity.
    UnresolvedInternState {
        /// The string whose prior canonical identity is unknown.
        value: String,
    },
    /// The compiler violated one of its own invariants.
    Internal(String),
}

impl CompileError {
    /// Return the byte range associated with this error, if available.
    pub fn range(&self) -> Option<TextRange> {
        match self {
            Self::Parse(error) => Some(error.location),
            Self::InvalidPattern { range, .. }
            | Self::InvalidSyntax { range, .. }
            | Self::InvalidDocstring { range, .. } => Some(*range),
            Self::InvalidFilename
            | Self::Decode(_)
            | Self::Unsupported(_)
            | Self::UnresolvedInternState { .. }
            | Self::Internal(_) => None,
        }
    }

    pub(crate) fn invalid_syntax(message: impl Into<String>, range: TextRange) -> Self {
        Self::InvalidSyntax {
            message: message.into(),
            range,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFilename => formatter.write_str("embedded null character"),
            Self::Decode(error) => write!(formatter, "failed to decode Python source: {error}"),
            Self::Parse(error) => write!(formatter, "failed to parse Python source: {error}"),
            Self::InvalidPattern { message, .. } | Self::InvalidSyntax { message, .. } => {
                write!(formatter, "failed to parse Python source: {message}")
            }
            Self::InvalidDocstring { message, .. } => formatter.write_str(message),
            Self::Unsupported(message) => formatter.write_str(message),
            Self::UnresolvedInternState { value } => {
                write!(formatter, "unresolved initial intern state for {value:?}")
            }
            Self::Internal(message) => write!(formatter, "internal compiler error: {message}"),
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::InvalidFilename
            | Self::InvalidPattern { .. }
            | Self::InvalidSyntax { .. }
            | Self::InvalidDocstring { .. }
            | Self::Unsupported(_)
            | Self::UnresolvedInternState { .. }
            | Self::Internal(_) => None,
        }
    }
}

/// Compile a Python module for CPython 3.14.5.
pub fn compile(source: &str, filename: &str) -> Result<CompiledModule, CompileError> {
    compile_with_options(source, filename, &CompileOptions::default())
}

/// Compile a Python module with a fixed initial CPython string intern state.
///
/// The state is retained by the result for marshal and `.pyc` serialization. It does
/// not execute source imports or accumulate interned strings across compilations.
pub fn compile_with_intern_state(
    source: &str,
    filename: &str,
    initial_intern_state: &InitialInternState,
) -> Result<CompiledModule, CompileError> {
    compile_with_options(
        source,
        filename,
        &CompileOptions {
            initial_intern_state: initial_intern_state.clone(),
            ..CompileOptions::default()
        },
    )
}

/// Compile a Python module using the selected target and initial string intern state.
pub fn compile_with_options(
    source: &str,
    filename: &str,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    if source.contains('\0') {
        return Err(CompileError::Decode(SourceDecodeError::NullByte));
    }
    if source.starts_with('\u{feff}') {
        // Only byte source can use a leading BOM as an encoding signature. Ruff accepts
        // it in UTF-8 text as well, so reject it before parsing already decoded source.
        return Err(CompileError::invalid_syntax(
            "invalid non-printable character U+FEFF",
            TextRange::new(TextSize::new(0), TextSize::new(3)),
        ));
    }
    let source = source::normalize_source(source);
    compile_normalized(&source, filename.into(), options)
}

/// Decode and compile Python source bytes for CPython 3.14.5.
///
/// This function performs no filesystem I/O. See [`decode_source`] for the accepted source
/// encoding and normalization behavior.
pub fn compile_bytes(source: &[u8], filename: &str) -> Result<CompiledModule, CompileError> {
    compile_bytes_with_options(source, filename, &CompileOptions::default())
}

/// Decode and compile source bytes with a fixed initial CPython string intern state.
///
/// This function performs no filesystem I/O. The state has the same meaning as in
/// [`compile_with_intern_state`].
pub fn compile_bytes_with_intern_state(
    source: &[u8],
    filename: &str,
    initial_intern_state: &InitialInternState,
) -> Result<CompiledModule, CompileError> {
    compile_bytes_with_options(
        source,
        filename,
        &CompileOptions {
            initial_intern_state: initial_intern_state.clone(),
            ..CompileOptions::default()
        },
    )
}

/// Decode and compile source bytes using the selected target and initial string intern state.
///
/// This function performs no filesystem I/O. See [`decode_source`] for source encoding behavior.
/// The initial intern state describes the state before byte-source decoding; by default,
/// standard codec imports and successful codec lookups contribute retained strings.
pub fn compile_bytes_with_options(
    source: &[u8],
    filename: &str,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    compile_byte_source(source, filename.into(), options)
}

/// Decode and compile source bytes with an operating-system filename and the selected options.
///
/// This function performs no filesystem I/O. Unlike [`compile_bytes_with_options`], it also
/// preserves filenames that are not valid Unicode, using Python's filesystem representation.
/// Paths containing a null character are rejected before the source is decoded.
pub fn compile_bytes_with_path_and_options(
    source: &[u8],
    filename: &Path,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    if filename.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CompileError::InvalidFilename);
    }
    compile_byte_source(source, CodeFilename::from_path(filename), options)
}

fn compile_byte_source(
    source: &[u8],
    filename: CodeFilename,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    let decoded = source::decode_source_with_codec(source).map_err(CompileError::Decode)?;
    if !options
        .initial_intern_state
        .models_standard_source_codec_effects()
    {
        return compile_normalized(&decoded.source, filename, options);
    }
    let strings = decoded.codec.map_or(&[][..], |codec| {
        target::source_codec_strings(options.python_version, codec)
    });
    if strings.is_empty() && decoded.registry_name.is_none() {
        return compile_normalized(&decoded.source, filename, options);
    }
    let state = options.initial_intern_state.with_additional_strings(
        strings
            .iter()
            .copied()
            .chain(decoded.registry_name.as_deref()),
    );
    compile_normalized(
        &decoded.source,
        filename,
        &CompileOptions {
            initial_intern_state: state,
            ..options.clone()
        },
    )
}

fn validate_filename(filename: &str) -> Result<(), CompileError> {
    if filename.contains('\0') {
        return Err(CompileError::InvalidFilename);
    }
    Ok(())
}

const STACK_SEGMENT_SIZE: usize = 2 * 1024 * 1024;
const STACK_RED_ZONE: usize = 256 * 1024;

thread_local! {
    static STACK_REENTRY_BYPASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Check before recursive AST operations so they can grow on the current thread.
#[inline]
fn stack_growth_required() -> bool {
    stack_growth_required_for(stacker::remaining_stack())
}

#[inline]
fn stack_growth_required_for(remaining: Option<usize>) -> bool {
    let bypass = STACK_REENTRY_BYPASS.with(|bypass| bypass.replace(false));
    if remaining.is_some_and(|remaining| remaining >= STACK_RED_ZONE) {
        return false;
    }
    !bypass
}

/// Reenter a guarded operation after growing its stack. Some platforms cannot
/// switch stacks or report the new bounds; allow that reentry to run once
/// rather than repeatedly invoking stack growth without making progress.
fn grow_stack<T>(callback: impl FnOnce() -> T) -> T {
    stacker::grow(STACK_SEGMENT_SIZE, || {
        if stacker::remaining_stack().is_some_and(|remaining| remaining >= STACK_RED_ZONE) {
            callback()
        } else {
            with_stack_reentry_bypass(callback)
        }
    })
}

fn with_stack_reentry_bypass<T>(callback: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            STACK_REENTRY_BYPASS.with(|bypass| bypass.set(self.0));
        }
    }
    let previous = STACK_REENTRY_BYPASS.with(|bypass| bypass.replace(true));
    let _restore = Restore(previous);
    callback()
}

/// Owns the parser output while allowing deeply nested expressions to be dropped safely.
struct ParsedModule(Option<ruff_python_parser::Parsed<ruff_python_ast::ModModule>>);

impl Drop for ParsedModule {
    fn drop(&mut self) {
        use ruff_python_ast::visitor::transformer::{Transformer, walk_expr};

        struct ClearExpressions;

        impl Transformer for ClearExpressions {
            fn visit_expr(&self, expression: &mut ruff_python_ast::Expr) {
                stacker::maybe_grow(STACK_RED_ZONE, STACK_SEGMENT_SIZE, || {
                    walk_expr(self, expression);
                    // Its children have already been cleared, so replacing the node does not
                    // recursively drop a potentially unbounded expression tree.
                    *expression = ruff_python_ast::Expr::NoneLiteral(
                        ruff_python_ast::ExprNoneLiteral::default(),
                    );
                });
            }
        }

        if let Some(parsed) = self.0.take() {
            let mut suite = parsed.into_suite();
            ClearExpressions.visit_body(&mut suite);
        }
    }
}

fn compile_normalized(
    source: &str,
    filename: CodeFilename,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    if stack_growth_required() {
        return grow_stack(|| compile_normalized(source, filename, options));
    }
    options
        .native_build_profile
        .validate(options.python_version)?;
    let target_version = match options.python_version {
        PythonVersion::Py312 => ruff_python_ast::PythonVersion::PY312,
        PythonVersion::Py313 => ruff_python_ast::PythonVersion::PY313,
        PythonVersion::Py314 => ruff_python_ast::PythonVersion::PY314,
        PythonVersion::Py315 => ruff_python_ast::PythonVersion::PY315,
    };
    let parse_options = ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Module)
        .with_target_version(target_version);
    delimiter_nesting::validate(source, &parse_options, options.python_version)?;
    let parsed = ruff_python_parser::parse_unchecked(source, parse_options.clone())
        .try_into_module()
        .ok_or_else(|| CompileError::Internal("parser did not return a module".to_string()))?;
    let parsed = ParsedModule(Some(parsed));
    let parsed = parsed.0.as_ref().expect("parsed module is present");
    let parse_error = parsed.errors().first();
    if let Some(range) = indentation::first_excess(parsed.tokens())
        && parse_error.is_none_or(|error| range.start() <= error.location.start())
    {
        if let Some(error) = yield_syntax::before_indentation_error(
            source,
            &parse_options,
            range,
            options.python_version,
        ) {
            return Err(error);
        }
        return Err(indentation::excess_error(range));
    }
    if let Some(error) = parse_error {
        if let Some(yield_error) = yield_syntax::before_parse_error(
            source,
            parsed.tokens(),
            parsed.errors(),
            parsed.unsupported_syntax_errors(),
            options.python_version,
        ) {
            return Err(yield_error);
        }
        return Err(CompileError::Parse(error.clone()));
    }
    if let Some(error) = yield_syntax::earlier_unsupported(
        source,
        parsed.tokens(),
        parsed.unsupported_syntax_errors(),
        options.python_version,
    ) {
        return Err(error);
    }
    yield_syntax::validate(source, parsed.tokens(), options.python_version)?;
    if let Some(error) = parsed.unsupported_syntax_errors().first() {
        return Err(CompileError::invalid_syntax(error.to_string(), error.range));
    }
    compiler::syntax::validate(source, parsed.suite(), options.python_version)?;
    let code = Compiler::module_with_profile(
        filename,
        source,
        parsed.tokens(),
        parsed.suite(),
        options.python_version,
        options.native_build_profile,
    )
    .compile_module(parsed.suite())?;
    marshal::validate_initial_intern_state(
        &code,
        options.python_version,
        &options.initial_intern_state,
    )?;
    Ok(CompiledModule {
        code,
        python_version: options.python_version,
        initial_intern_state: options.initial_intern_state.clone(),
    })
}

/// Compile Python source bytes directly to a CPython 3.14.5 `.pyc` file.
///
/// This function performs no filesystem I/O. `source_mtime` is written to the timestamp header,
/// and its source-size field uses the original byte length before decoding or normalization.
pub fn compile_bytes_to_pyc(
    source: &[u8],
    filename: &str,
    source_mtime: u32,
) -> Result<Vec<u8>, CompileError> {
    validate_filename(filename)?;
    let source_size = u32::try_from(source.len()).map_err(|_| {
        CompileError::Unsupported("Python source exceeds the 4 GiB `.pyc` limit".to_string())
    })?;
    Ok(compile_bytes(source, filename)?.to_timestamp_pyc(source_mtime, source_size))
}

/// Compile a Python module directly to a timestamp-invalidated CPython 3.14.5 `.pyc` file.
pub fn compile_to_pyc(
    source: &str,
    filename: &str,
    source_mtime: u32,
) -> Result<Vec<u8>, CompileError> {
    validate_filename(filename)?;
    let source_size = u32::try_from(source.len()).map_err(|_| {
        CompileError::Unsupported("Python source exceeds the 4 GiB `.pyc` limit".to_string())
    })?;
    Ok(compile(source, filename)?.to_timestamp_pyc(source_mtime, source_size))
}
