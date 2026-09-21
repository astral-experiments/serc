use std::error::Error;
use std::fmt;
use std::str::FromStr;

use crate::{CPYTHON_TARGET, CpythonTarget, InitialInternState, target};

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

/// Configuration retained by a compiled module for target-specific serialization.
#[derive(Clone, Debug, Default)]
pub struct CompileOptions {
    /// The Python language and bytecode version to target. Defaults to Python 3.14.
    pub python_version: PythonVersion,
    /// Canonical strings present before compiling. See [`InitialInternState`] for state modes.
    pub initial_intern_state: InitialInternState,
}

#[cfg(test)]
mod tests {
    use super::{CompileOptions, PythonVersion};
    use crate::{
        CPYTHON_TARGET, InitialInternState, compile, compile_bytes,
        compile_bytes_with_intern_state, compile_bytes_with_options, compile_with_intern_state,
        compile_with_options,
    };

    #[test]
    fn version_configuration_selects_exact_target() {
        assert_eq!(
            CompileOptions::default().python_version,
            PythonVersion::Py314
        );
        assert_eq!(PythonVersion::Py314.target(), CPYTHON_TARGET);
        assert_eq!(
            "3.14".parse::<PythonVersion>().unwrap(),
            PythonVersion::Py314
        );
        assert_eq!(
            "3.15".parse::<PythonVersion>().unwrap(),
            PythonVersion::Py315
        );
        assert_eq!(PythonVersion::Py315.to_string(), "3.15");
        assert_eq!(PythonVersion::Py315.target().tag, "v3.15.0rc2");
        assert_eq!(PythonVersion::Py315.target().magic_hex(), "520e0d0a");
        for (version, spelling, tag, commit, magic) in [
            (
                PythonVersion::Py312,
                "3.12",
                "v3.12.14",
                "2abcf904b8dac8c999d2b3aac76681abb333798a",
                "cb0d0d0a",
            ),
            (
                PythonVersion::Py313,
                "3.13",
                "v3.13.15",
                "4061bc4c35f7c26f25264666d4ba083b93d2f6f9",
                "f30d0d0a",
            ),
        ] {
            assert_eq!(spelling.parse::<PythonVersion>().unwrap(), version);
            assert_eq!(version.to_string(), spelling);
            assert_eq!(version.target().tag, tag);
            assert_eq!(version.target().commit, commit);
            assert_eq!(version.target().magic_hex(), magic);
        }
        assert!("3.11".parse::<PythonVersion>().is_err());
        assert!("3.15.1".parse::<PythonVersion>().is_err());
    }

    #[test]
    fn compiled_modules_retain_independent_targets() {
        let source = "answer = 42\n";
        let default = compile(source, "module.py").unwrap();
        let legacy = default.to_timestamp_pyc(123, 12);
        for python_version in [
            PythonVersion::Py315,
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            let module = compile_with_options(source, "module.py", &options).unwrap();
            assert_eq!(module.python_version(), python_version);
            assert_eq!(module.target(), python_version.target());
            let pyc = module.to_timestamp_pyc(123, 12);
            assert_eq!(&pyc[..4], &python_version.target().magic_number);
            if python_version == PythonVersion::Py315 {
                assert_eq!(pyc[16], b'c');
                assert_eq!(module.marshal()[0], b'c' | 0x80);
            } else {
                assert_eq!(&pyc[16..], module.marshal());
            }
            assert_eq!(
                compile_bytes_with_options(source.as_bytes(), "module.py", &options)
                    .unwrap()
                    .to_timestamp_pyc(123, 12),
                pyc
            );
        }
        assert_eq!(default.to_timestamp_pyc(123, 12), legacy);
        assert_eq!(
            compile_bytes(source.as_bytes(), "module.py")
                .unwrap()
                .to_timestamp_pyc(123, 12),
            legacy
        );
    }

    #[test]
    fn legacy_intern_state_entrypoints_keep_python_314_default() {
        let source = "result = key in {'aggregate_class', 'custom_interned'}\n";
        let state = InitialInternState::new(["aggregate_class", "custom_interned"]);
        let options = CompileOptions {
            initial_intern_state: state.clone(),
            ..CompileOptions::default()
        };
        let expected = compile_with_options(source, "module.py", &options)
            .unwrap()
            .marshal();
        assert_eq!(
            compile_with_intern_state(source, "module.py", &state)
                .unwrap()
                .marshal(),
            expected
        );
        assert_eq!(
            compile_bytes_with_intern_state(source.as_bytes(), "module.py", &state)
                .unwrap()
                .marshal(),
            expected
        );
    }
}
