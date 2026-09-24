//! Compile Python source to CPython bytecode without starting a Python interpreter.
//!
//! Select Python 3.12, 3.13, 3.14, or 3.15 with [`CompileOptions`]; Python 3.14 is the default.
//! [`PythonVersion::target`] identifies each pinned CPython release. Bytecode, code objects,
//! marshal, and `.pyc` formats are implementation details tied to those exact releases.

mod assembler;
mod code_object;
mod compiler;
mod config;
mod initial_intern_state;
mod marshal;
mod source;
mod target;

use std::error::Error;
use std::fmt;

use code_object::CodeObject;
use compiler::Compiler;
pub use config::{CompileOptions, ParsePythonVersionError, PythonVersion};
pub use initial_intern_state::InitialInternState;
use ruff_text_size::TextRange;
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
        let mut output = Vec::with_capacity(16 + self.code.bytecode.len());
        output.extend_from_slice(&self.target().magic_number);
        output.extend_from_slice(&0_u32.to_le_bytes());
        output.extend_from_slice(&source_mtime.to_le_bytes());
        output.extend_from_slice(&source_size.to_le_bytes());
        marshal::encode_pyc_code_into_with_options(
            &self.code,
            self.python_version,
            &self.initial_intern_state,
            &mut output,
        );
        output
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
            Self::InvalidPattern { range, .. } | Self::InvalidSyntax { range, .. } => Some(*range),
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
    let source = source::normalize_source(source);
    compile_normalized(&source, filename, options)
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
pub fn compile_bytes_with_options(
    source: &[u8],
    filename: &str,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    validate_filename(filename)?;
    let source = decode_source(source).map_err(CompileError::Decode)?;
    compile_normalized(&source, filename, options)
}

fn validate_filename(filename: &str) -> Result<(), CompileError> {
    if filename.contains('\0') {
        return Err(CompileError::InvalidFilename);
    }
    Ok(())
}

fn compile_normalized(
    source: &str,
    filename: &str,
    options: &CompileOptions,
) -> Result<CompiledModule, CompileError> {
    let target_version = match options.python_version {
        PythonVersion::Py312 => ruff_python_ast::PythonVersion::PY312,
        PythonVersion::Py313 => ruff_python_ast::PythonVersion::PY313,
        PythonVersion::Py314 => ruff_python_ast::PythonVersion::PY314,
        PythonVersion::Py315 => ruff_python_ast::PythonVersion::PY315,
    };
    let parse_options = ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Module)
        .with_target_version(target_version);
    let parsed = ruff_python_parser::parse(source, parse_options)
        .map_err(CompileError::Parse)?
        .try_into_module()
        .ok_or_else(|| CompileError::Internal("parser did not return a module".to_string()))?;
    if let Some(error) = parsed.unsupported_syntax_errors().first() {
        return Err(CompileError::invalid_syntax(error.to_string(), error.range));
    }
    compiler::syntax::validate(source, parsed.suite(), options.python_version)?;
    let code = Compiler::module_with_python_version(
        filename,
        source,
        parsed.tokens(),
        parsed.suite(),
        options.python_version,
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
