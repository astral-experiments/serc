//! Target identities, semantic opcodes, and marshal formats shared by serc's compiler and decompiler.
#[cfg(feature = "read")]
pub mod instructions;
pub mod marshal;
pub mod target;

use siphasher::sip::SipHasher13;
use std::{error::Error, fmt, str::FromStr};

/// The Python language and bytecode version used for compilation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PythonVersion {
    /// CPython 3.12.14.
    Py312,
    /// CPython 3.13.15.
    Py313,
    /// CPython 3.14.5.
    #[default]
    Py314,
    /// CPython 3.15.0rc2.
    Py315,
}

impl PythonVersion {
    /// Return the exact CPython release and bytecode format for this version.
    pub const fn target(self) -> CpythonTarget {
        match self {
            Self::Py312 => CpythonTarget {
                implementation: target::cpython_3_12_14::TARGET_IMPLEMENTATION,
                version: target::cpython_3_12_14::TARGET_VERSION,
                magic_number: target::cpython_3_12_14::TARGET_MAGIC_NUMBER,
                tag: target::cpython_3_12_14::TARGET_TAG,
                commit: target::cpython_3_12_14::TARGET_COMMIT,
            },
            Self::Py313 => CpythonTarget {
                implementation: target::cpython_3_13_15::TARGET_IMPLEMENTATION,
                version: target::cpython_3_13_15::TARGET_VERSION,
                magic_number: target::cpython_3_13_15::TARGET_MAGIC_NUMBER,
                tag: target::cpython_3_13_15::TARGET_TAG,
                commit: target::cpython_3_13_15::TARGET_COMMIT,
            },
            Self::Py314 => CPYTHON_TARGET,
            Self::Py315 => CpythonTarget {
                implementation: target::cpython_3_15_0rc2::TARGET_IMPLEMENTATION,
                version: target::cpython_3_15_0rc2::TARGET_VERSION,
                magic_number: target::cpython_3_15_0rc2::TARGET_MAGIC_NUMBER,
                tag: target::cpython_3_15_0rc2::TARGET_TAG,
                commit: target::cpython_3_15_0rc2::TARGET_COMMIT,
            },
        }
    }
}

impl fmt::Display for PythonVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Py312 => "3.12",
            Self::Py313 => "3.13",
            Self::Py314 => "3.14",
            Self::Py315 => "3.15",
        })
    }
}

impl FromStr for PythonVersion {
    type Err = ParsePythonVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "3.12" => Ok(Self::Py312),
            "3.13" => Ok(Self::Py313),
            "3.14" => Ok(Self::Py314),
            "3.15" => Ok(Self::Py315),
            _ => Err(ParsePythonVersionError(value.to_owned())),
        }
    }
}

/// An unsupported Python version supplied to [`PythonVersion::from_str`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsePythonVersionError(String);

impl fmt::Display for ParsePythonVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported Python version {:?}; expected 3.12, 3.13, 3.14, or 3.15",
            self.0
        )
    }
}

impl Error for ParsePythonVersionError {}

/// Python's bytecode optimization level.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OptimizationLevel {
    /// Retain assertions and docstrings, and evaluate `__debug__` as true.
    #[default]
    Zero,
    /// Remove assertions and evaluate `__debug__` as false, like Python's `-O`.
    One,
    /// Also remove docstrings, like Python's `-OO`.
    Two,
}

impl fmt::Display for OptimizationLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Zero => "0",
            Self::One => "1",
            Self::Two => "2",
        })
    }
}

impl FromStr for OptimizationLevel {
    type Err = ParseOptimizationLevelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "0" => Ok(Self::Zero),
            "1" => Ok(Self::One),
            "2" => Ok(Self::Two),
            _ => Err(ParseOptimizationLevelError(value.to_owned())),
        }
    }
}

/// An unsupported optimization level supplied to [`OptimizationLevel::from_str`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOptimizationLevelError(String);

impl fmt::Display for ParseOptimizationLevelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported optimization level {:?}; expected 0, 1, or 2",
            self.0
        )
    }
}

impl Error for ParseOptimizationLevelError {}

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
    /// Hash original source bytes for this target's hash-invalidated cache header.
    pub fn source_hash(self, source: &[u8]) -> [u8; 8] {
        // CPython's _imp.source_hash uses SipHash-1-3 keyed by the target magic.
        let key = u64::from(u32::from_le_bytes(self.magic_number));
        SipHasher13::new_with_keys(key, 0)
            .hash(source)
            .to_le_bytes()
    }

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
