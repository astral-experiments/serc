//! Compile Python source to CPython bytecode without starting a Python interpreter.
//!
//! Select Python 3.12, 3.13, 3.14, or 3.15 with [`CompileOptions`]; Python 3.14 is the default.
//! [`PythonVersion::target`] identifies each pinned CPython release. Bytecode, code objects,
//! marshal, and `.pyc` formats are implementation details tied to those exact releases.

mod assembler;
mod code_object;
mod compiler;
mod config;
mod decimal_literals;
mod delimiter_nesting;
mod indentation;
mod initial_intern_state;
mod marshal;
mod paramspec_default;
mod python_312_starred_defaults;
mod source;
mod stack;
mod target;
mod type_alias_syntax;
mod yield_from_syntax;
mod yield_syntax;

use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::path::Path;

use code_object::{CodeFilename, CodeObject};
use compiler::Compiler;
pub use config::{
    CompileOptions, NativeBuildProfile, OptimizationLevel, ParseOptimizationLevelError,
    ParsePythonVersionError, PythonVersion,
};
pub use initial_intern_state::InitialInternState;
use ruff_text_size::{Ranged, TextRange, TextSize};
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
    /// CPython raises `ValueError` when formatting an integer exceeds the selected limit.
    IntegerStringConversionLimit {
        /// The maximum number of decimal digits configured at the formatting event.
        max_digits: u32,
    },
    /// The caller has not established the integer-to-string limit needed for this diagnostic.
    UnresolvedIntegerStringLimit,
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
            | Self::IntegerStringConversionLimit { .. }
            | Self::UnresolvedIntegerStringLimit
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
            Self::IntegerStringConversionLimit { max_digits } => write!(
                formatter,
                "Exceeds the limit ({max_digits} digits) for integer string conversion; use sys.set_int_max_str_digits() to increase the limit"
            ),
            Self::UnresolvedIntegerStringLimit => {
                formatter.write_str("unresolved integer-to-string conversion limit")
            }
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
            | Self::IntegerStringConversionLimit { .. }
            | Self::UnresolvedIntegerStringLimit
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

/// Compile a Python module using the selected target, optimization level, and initial intern state.
pub fn compile_with_options(
    source: &str,
    filename: &str,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    compile_text_source(source, filename.into(), options, None, None)
}

/// Compile source with a proved CPython integer-string limit for duplicate mapping keys.
///
/// `limit` is zero for an unlimited interpreter, or a value from 640 through `i32::MAX`.
/// It must be the limit at the key-formatting event, after any callbacks have run. The caller
/// must also establish that earlier callbacks return normally and do not replace the result.
/// Decimal parsing has a separate limit, set by [`compile_with_options_and_integer_limits`];
/// duplicate-key diagnostics remain unsupported when a decimal token could depend on an
/// unestablished parsing limit. Large values that can invoke `_pylong` also remain unsupported
/// unless CPython's earlier check already proves a conversion-limit error.
pub fn compile_with_options_and_mapping_key_limit(
    source: &str,
    filename: &str,
    options: &CompileOptions,
    limit: u32,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    validate_integer_string_limit(limit)?;
    compile_text_source(source, filename.into(), options, None, Some(limit))
}

/// Compile source with explicit parsing and optional duplicate-key formatting limits.
///
/// `parse_limit` is the CPython integer-string conversion limit in effect when decimal
/// literals are parsed. Zero disables the limit; other values must be from 640 through
/// `i32::MAX`. Unlike [`compile_with_options`], this establishes the parsing limit for
/// diagnostics that could otherwise depend on an unknown caller setting.
///
/// `mapping_key_limit` is independent: `Some(limit)` establishes the limit when a
/// duplicate mapping key is formatted, with the same callback requirements and
/// limitations as [`compile_with_options_and_mapping_key_limit`]. `None` leaves that
/// formatting limit unknown.
pub fn compile_with_options_and_integer_limits(
    source: &str,
    filename: &str,
    options: &CompileOptions,
    parse_limit: u32,
    mapping_key_limit: Option<u32>,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    validate_integer_limits(parse_limit, mapping_key_limit)?;
    compile_text_source(
        source,
        filename.into(),
        options,
        Some(parse_limit),
        mapping_key_limit,
    )
}

fn compile_text_source(
    source: &str,
    filename: CodeFilename,
    options: &CompileOptions,
    parse_int_max_str_digits: Option<u32>,
    mapping_key_int_max_str_digits: Option<u32>,
) -> Result<CompiledModule, CompileError> {
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
    compile_normalized(
        &source,
        filename,
        options,
        parse_int_max_str_digits,
        mapping_key_int_max_str_digits,
    )
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

/// Decode and compile source bytes using the selected target, optimization level, and intern state.
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
    compile_byte_source(source, filename.into(), options, None, None)
}

/// Decode and compile source bytes with a proved duplicate-key integer-string limit.
///
/// The limit and callback requirements are the same as
/// [`compile_with_options_and_mapping_key_limit`].
pub fn compile_bytes_with_options_and_mapping_key_limit(
    source: &[u8],
    filename: &str,
    options: &CompileOptions,
    limit: u32,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    validate_integer_string_limit(limit)?;
    compile_byte_source(source, filename.into(), options, None, Some(limit))
}

/// Decode and compile source bytes with explicit parsing and optional formatting limits.
///
/// The limits and callback requirements are the same as
/// [`compile_with_options_and_integer_limits`].
pub fn compile_bytes_with_options_and_integer_limits(
    source: &[u8],
    filename: &str,
    options: &CompileOptions,
    parse_limit: u32,
    mapping_key_limit: Option<u32>,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    validate_integer_limits(parse_limit, mapping_key_limit)?;
    compile_byte_source(
        source,
        filename.into(),
        options,
        Some(parse_limit),
        mapping_key_limit,
    )
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
    compile_byte_source(
        source,
        CodeFilename::from_path(filename),
        options,
        None,
        None,
    )
}

/// Decode and compile source bytes with an OS filename and proved duplicate-key limit.
///
/// The limit and callback requirements are the same as
/// [`compile_with_options_and_mapping_key_limit`].
pub fn compile_bytes_with_path_options_and_mapping_key_limit(
    source: &[u8],
    filename: &Path,
    options: &CompileOptions,
    limit: u32,
) -> Result<CompiledModule, CompileError> {
    if filename.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CompileError::InvalidFilename);
    }
    validate_integer_string_limit(limit)?;
    compile_byte_source(
        source,
        CodeFilename::from_path(filename),
        options,
        None,
        Some(limit),
    )
}

/// Decode source bytes with an OS filename and explicit parsing and formatting limits.
///
/// The limits and callback requirements are the same as
/// [`compile_with_options_and_integer_limits`]. Filenames are preserved as in
/// [`compile_bytes_with_path_and_options`].
pub fn compile_bytes_with_path_options_and_integer_limits(
    source: &[u8],
    filename: &Path,
    options: &CompileOptions,
    parse_limit: u32,
    mapping_key_limit: Option<u32>,
) -> Result<CompiledModule, CompileError> {
    if filename.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CompileError::InvalidFilename);
    }
    validate_integer_limits(parse_limit, mapping_key_limit)?;
    compile_byte_source(
        source,
        CodeFilename::from_path(filename),
        options,
        Some(parse_limit),
        mapping_key_limit,
    )
}

fn compile_byte_source(
    source: &[u8],
    filename: CodeFilename,
    options: &CompileOptions,
    parse_int_max_str_digits: Option<u32>,
    mapping_key_int_max_str_digits: Option<u32>,
) -> Result<CompiledModule, CompileError> {
    let decoded = source::decode_source_with_codec(source).map_err(CompileError::Decode)?;
    let mut options = Cow::Borrowed(options);
    if options
        .initial_intern_state
        .models_standard_source_codec_effects()
    {
        let strings = decoded.codec.map_or(&[][..], |codec| {
            target::source_codec_strings(options.python_version, codec)
        });
        if !strings.is_empty() || decoded.registry_name.is_some() {
            let state = options.initial_intern_state.with_additional_strings(
                strings
                    .iter()
                    .copied()
                    .chain(decoded.registry_name.as_deref()),
            );
            options.to_mut().initial_intern_state = state;
        }
    }
    compile_normalized(
        &decoded.source,
        filename,
        &options,
        parse_int_max_str_digits,
        mapping_key_int_max_str_digits,
    )
}

fn validate_filename(filename: &str) -> Result<(), CompileError> {
    if filename.contains('\0') {
        return Err(CompileError::InvalidFilename);
    }
    Ok(())
}

fn validate_integer_limits(
    parse_limit: u32,
    mapping_key_limit: Option<u32>,
) -> Result<(), CompileError> {
    validate_integer_string_limit(parse_limit)?;
    if let Some(limit) = mapping_key_limit {
        validate_integer_string_limit(limit)?;
    }
    Ok(())
}

fn validate_integer_string_limit(limit: u32) -> Result<(), CompileError> {
    if limit != 0 && !(640..=2_147_483_647).contains(&limit) {
        return Err(CompileError::Unsupported(
            "integer-to-string limit must be zero or between 640 and i32::MAX".to_owned(),
        ));
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

        struct ClearExpressions {
            shallow_drop: bool,
        }

        impl Transformer for ClearExpressions {
            fn visit_expr(&self, expression: &mut ruff_python_ast::Expr) {
                // Leaves cannot recurse during destruction. Let their parent drop them instead
                // of checking the stack and replacing every scalar with another scalar.
                if matches!(
                    expression,
                    ruff_python_ast::Expr::Name(_)
                        | ruff_python_ast::Expr::NumberLiteral(_)
                        | ruff_python_ast::Expr::StringLiteral(_)
                        | ruff_python_ast::Expr::BytesLiteral(_)
                        | ruff_python_ast::Expr::BooleanLiteral(_)
                        | ruff_python_ast::Expr::NoneLiteral(_)
                        | ruff_python_ast::Expr::EllipsisLiteral(_)
                ) {
                    return;
                }
                // A short source range bounds the depth of an unmodified, successfully parsed
                // expression. With a full stack segment available it can be dropped directly;
                // long chains and callers near their stack limit retain guarded dismantling.
                if self.shallow_drop && expression.range().len() <= TextSize::new(256) {
                    return;
                }
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
            let shallow_drop = parsed.errors().is_empty()
                && stacker::remaining_stack()
                    .is_some_and(|remaining| remaining >= STACK_SEGMENT_SIZE);
            let mut suite = parsed.into_suite();
            ClearExpressions { shallow_drop }.visit_body(&mut suite);
        }
    }
}

// A Python decimal integer may fail while parsing, before mapping-key lowering. The
// parsing limit is separate from the limit supplied for formatting. This raw scan
// overapproximates a long decimal token even when Ruff stops on an earlier error.
fn may_depend_on_decimal_parse(source: &str) -> bool {
    if source.len() <= 640 || !source.contains("match") || !source.contains("case") {
        return false;
    }
    let mut digits = 0_usize;
    for byte in source.bytes() {
        if byte.is_ascii_digit() {
            digits += 1;
            if digits > 640 {
                return true;
            }
        } else if byte != b'_' {
            digits = 0;
        }
    }
    false
}

fn unknown_decimal_parse() -> CompileError {
    CompileError::Unsupported("decimal integer parsing limit is not known".to_owned())
}

fn has_large_decimal_token(source: &str, tokens: &ruff_python_ast::token::Tokens) -> bool {
    use ruff_python_ast::token::TokenKind;
    tokens.iter().any(|token| {
        if token.kind() != TokenKind::Int {
            return false;
        }
        let spelling = &source[token.range()];
        if spelling.starts_with("0x")
            || spelling.starts_with("0X")
            || spelling.starts_with("0o")
            || spelling.starts_with("0O")
            || spelling.starts_with("0b")
            || spelling.starts_with("0B")
        {
            return false;
        }
        spelling.bytes().filter(u8::is_ascii_digit).count() > 640
    })
}

fn compile_normalized(
    source: &str,
    filename: CodeFilename,
    options: &CompileOptions,
    parse_int_max_str_digits: Option<u32>,
    mapping_key_int_max_str_digits: Option<u32>,
) -> Result<CompiledModule, CompileError> {
    if stack_growth_required() {
        return grow_stack(|| {
            compile_normalized(
                source,
                filename,
                options,
                parse_int_max_str_digits,
                mapping_key_int_max_str_digits,
            )
        });
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
    let may_depend_on_decimal_parse =
        parse_int_max_str_digits.is_none() && may_depend_on_decimal_parse(source);
    if options.python_version <= PythonVersion::Py313
        && let Err(error) =
            delimiter_nesting::validate(source, &parse_options, options.python_version)
    {
        return Err(if may_depend_on_decimal_parse {
            unknown_decimal_parse()
        } else {
            error
        });
    }
    let parsed = ruff_python_parser::parse_unchecked(source, parse_options.clone())
        .try_into_module()
        .ok_or_else(|| CompileError::Internal("parser did not return a module".to_string()))?;
    let parsed = ParsedModule(Some(parsed));
    let parsed = parsed.0.as_ref().expect("parsed module is present");
    // Ruff grows its parser stack for deeply nested expressions. Successful modern parses can
    // reuse its tokens; malformed input still uses the independent tokenizer before diagnostics.
    if options.python_version >= PythonVersion::Py314
        && let Err(error) = delimiter_nesting::validate_parsed(
            source,
            &parse_options,
            options.python_version,
            parsed,
        )
    {
        return Err(if may_depend_on_decimal_parse {
            unknown_decimal_parse()
        } else {
            error
        });
    }
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
    let prefer_type_alias_error = |error| {
        type_alias_syntax::before_error(
            source,
            parsed.suite(),
            parsed.tokens(),
            parsed.errors(),
            parsed.unsupported_syntax_errors(),
            options.python_version,
            &error,
        )
        .unwrap_or(error)
    };
    if let Some(error) = parse_error {
        if may_depend_on_decimal_parse {
            return Err(unknown_decimal_parse());
        }
        let error = if let Some(yield_error) = yield_syntax::before_parse_error(
            source,
            parsed.tokens(),
            parsed.errors(),
            parsed.unsupported_syntax_errors(),
            options.python_version,
        ) {
            yield_error
        } else if parsed.errors().len() == 1
            && parsed.unsupported_syntax_errors().is_empty()
            && matches!(
                error.error,
                ruff_python_parser::ParseErrorType::InvalidNamedAssignmentTarget
            )
            && is_attribute_named_expression_target(parsed.suite(), error.location)
        {
            CompileError::invalid_syntax(
                "cannot use assignment expressions with attribute",
                error.location,
            )
        } else if let Some((message, range)) = yield_from_syntax::invalid_tuple_diagnostic(parsed) {
            CompileError::invalid_syntax(message, range)
        } else {
            CompileError::Parse(error.clone())
        };
        return Err(prefer_type_alias_error(error));
    }
    if let Some(error) = yield_syntax::earlier_unsupported(
        source,
        parsed.tokens(),
        parsed.unsupported_syntax_errors(),
        options.python_version,
    ) {
        return Err(prefer_type_alias_error(error));
    }
    if let Err(error) = yield_syntax::validate(source, parsed.tokens(), options.python_version) {
        return Err(prefer_type_alias_error(error));
    }
    decimal_literals::validate(
        source,
        parsed.tokens(),
        parsed.suite(),
        parsed.unsupported_syntax_errors(),
        parse_int_max_str_digits.unwrap_or(4300),
    )?;
    if options.python_version == PythonVersion::Py312 {
        // Both validators can find errors in one module; CPython reports the first.
        let starred = python_312_starred_defaults::validate(parsed);
        let paramspec = (source.contains("**") && source.contains('['))
            .then(|| paramspec_default::validate(parsed.suite(), parsed.tokens()))
            .flatten()
            .filter(|error| {
                parsed
                    .unsupported_syntax_errors()
                    .first()
                    .is_none_or(|existing| {
                        error
                            .range()
                            .expect("ParamSpec diagnostic has a range")
                            .start()
                            < existing.range.start()
                    })
            });
        if let Some(error) = [starred, paramspec]
            .into_iter()
            .flatten()
            .min_by_key(|error| error.range().expect("diagnostic has a range").start())
        {
            return Err(error);
        }
    }
    if let Some(error) = parsed.unsupported_syntax_errors().first() {
        if may_depend_on_decimal_parse {
            return Err(unknown_decimal_parse());
        }
        return Err(prefer_type_alias_error(CompileError::invalid_syntax(
            error.to_string(),
            error.range,
        )));
    }
    if may_depend_on_decimal_parse
        && has_large_decimal_token(source, parsed.tokens())
        && compiler::syntax::may_have_duplicate_mapping_key(parsed.suite())
    {
        return Err(unknown_decimal_parse());
    }
    compiler::syntax::validate_with_integer_limit(
        source,
        parsed.suite(),
        options.python_version,
        options.optimization_level,
        mapping_key_int_max_str_digits,
    )?;
    let optimized_body = (options.optimization_level != OptimizationLevel::Zero).then(|| {
        compiler::optimization::prepare(
            parsed.suite(),
            options.optimization_level,
            options.python_version,
        )
    });
    let body = optimized_body
        .as_ref()
        .map_or_else(|| parsed.suite(), |optimized| &optimized.body);
    let code = Compiler::module_with_scope_body(
        filename,
        source,
        parsed.tokens(),
        body,
        parsed.suite(),
        options.python_version,
        options.native_build_profile,
        optimized_body.as_ref(),
    )
    .compile_module(body)?;
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

/// Identify the target in Ruff's recovered AST, retaining its original source range.
fn is_attribute_named_expression_target(
    suite: &[ruff_python_ast::Stmt],
    target_range: TextRange,
) -> bool {
    use ruff_python_ast::visitor::{Visitor, walk_expr, walk_stmt};
    use ruff_python_ast::{Expr, Stmt};
    use ruff_text_size::Ranged;

    struct Finder {
        target_range: TextRange,
        found: bool,
    }

    impl<'a> Visitor<'a> for Finder {
        fn visit_stmt(&mut self, statement: &'a Stmt) {
            if self.found {
                return;
            }
            if stack_growth_required() {
                return grow_stack(|| self.visit_stmt(statement));
            }
            walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'a Expr) {
            if self.found {
                return;
            }
            if stack_growth_required() {
                return grow_stack(|| self.visit_expr(expression));
            }
            if let Expr::Named(named) = expression
                && named.target.range() == self.target_range
                && matches!(named.target.as_ref(), Expr::Attribute(_))
            {
                self.found = true;
            } else {
                walk_expr(self, expression);
            }
        }
    }

    let mut finder = Finder {
        target_range,
        found: false,
    };
    finder.visit_body(suite);
    finder.found
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

#[cfg(test)]
mod tests {
    use super::may_depend_on_decimal_parse;

    #[test]
    fn decimal_prescan_keeps_digit_and_keyword_boundaries() {
        for prefix in ["match case ", "match ", "case ", ""] {
            for digits in [0, 630, 631, 639, 640, 641, 4096] {
                for digit in ["1", "1_"] {
                    let source = format!("{prefix}{}", digit.repeat(digits));
                    assert_eq!(
                        may_depend_on_decimal_parse(&source),
                        prefix == "match case " && digits > 640,
                    );
                }
            }
        }
        let first = "1".repeat(320);
        let second = "1".repeat(321);
        assert!(!may_depend_on_decimal_parse(&format!(
            "match case {first}x{second}"
        )));
        assert!(may_depend_on_decimal_parse(&format!(
            "match case {first}_{second}"
        )));
    }
}
