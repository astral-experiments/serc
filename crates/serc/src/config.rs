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

/// Floating-point instruction ordering for an explicitly selected native CPython build.
///
/// `Default` preserves Serc's existing target-dependent arithmetic. The named
/// profiles select instruction ordering of the exact CPython 3.14.5 Linux x86-64
/// artifacts, including the sign of folded NaNs. They require a GNU/Linux x86-64
/// host. Selection does not inspect or authenticate an installed interpreter.
///
/// General powers still use the host math library. Callers must qualify the
/// selected artifact in the same math-library and floating-point environment;
/// a matching Python version, compiler name, or profile alone does not establish
/// byte-for-byte compatibility. See `docs/cpython-build-profiles.md` for the
/// qualified artifact identities and environment requirements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NativeBuildProfile {
    #[default]
    Default,
    /// actions/python-versions 3.14.5-25647354415, built with GCC 13.3.0.
    Cpython3145ActionsGcc,
    /// uv CPython 3.14.5, built June 2, 2026 with Clang 22.1.3.
    Cpython3145UvClang,
}

impl NativeBuildProfile {
    pub(crate) fn validate(self, version: PythonVersion) -> Result<(), crate::CompileError> {
        if self != Self::Default
            && (version != PythonVersion::Py314
                || !cfg!(all(
                    target_os = "linux",
                    target_arch = "x86_64",
                    target_env = "gnu"
                )))
        {
            return Err(crate::CompileError::Unsupported(format!(
                "native build profile {self} requires Python 3.14.5 on GNU/Linux x86-64"
            )));
        }
        Ok(())
    }
}

impl fmt::Display for NativeBuildProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Default => "default",
            Self::Cpython3145ActionsGcc => "cpython-3.14.5-actions-gcc-13.3-linux-x86_64",
            Self::Cpython3145UvClang => "cpython-3.14.5-uv-clang-22.1.3-linux-x86_64",
        })
    }
}

impl FromStr for NativeBuildProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "default" => Ok(Self::Default),
            "cpython-3.14.5-actions-gcc-13.3-linux-x86_64" => Ok(Self::Cpython3145ActionsGcc),
            "cpython-3.14.5-uv-clang-22.1.3-linux-x86_64" => Ok(Self::Cpython3145UvClang),
            _ => Err(format!("unsupported native build profile {value:?}")),
        }
    }
}

/// Configuration retained by a compiled module for target-specific serialization.
#[derive(Clone, Debug, Default)]
pub struct CompileOptions {
    /// The Python language and bytecode version to target. Defaults to Python 3.14.
    pub python_version: PythonVersion,
    /// Native arithmetic rules used before constants are pooled. Defaults to existing behavior.
    pub native_build_profile: NativeBuildProfile,
    /// Canonical strings present before compiling. See [`InitialInternState`] for state modes.
    pub initial_intern_state: InitialInternState,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CompileOptions, PythonVersion};
    use crate::{
        CPYTHON_TARGET, InitialInternState, compile, compile_bytes,
        compile_bytes_with_intern_state, compile_bytes_with_options,
        compile_bytes_with_path_and_options, compile_with_intern_state, compile_with_options,
    };

    #[test]
    fn native_profiles_reject_unrelated_targets() {
        use super::NativeBuildProfile;
        for native_build_profile in [
            NativeBuildProfile::Cpython3145ActionsGcc,
            NativeBuildProfile::Cpython3145UvClang,
        ] {
            assert_eq!(
                native_build_profile.to_string().parse(),
                Ok(native_build_profile)
            );
            if !cfg!(all(
                target_os = "linux",
                target_arch = "x86_64",
                target_env = "gnu"
            )) {
                let options = CompileOptions {
                    native_build_profile,
                    ..CompileOptions::default()
                };
                assert!(matches!(
                    compile_with_options("pass\n", "profile.py", &options),
                    Err(crate::CompileError::Unsupported(_))
                ));
            }
            for python_version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py315,
            ] {
                let options = CompileOptions {
                    python_version,
                    native_build_profile,
                    ..CompileOptions::default()
                };
                assert!(matches!(
                    compile_with_options("pass\n", "profile.py", &options),
                    Err(crate::CompileError::Unsupported(_))
                ));
            }
        }
        assert!("cpython-3.14.5".parse::<NativeBuildProfile>().is_err());
        assert_eq!(NativeBuildProfile::default(), NativeBuildProfile::Default);
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    fn native_profiles_reach_nested_constants_before_pooling() {
        use super::NativeBuildProfile;
        use crate::code_object::Constant;

        fn complex_bits(value: &Constant, output: &mut Vec<(u64, u64)>) {
            match value.clone().into_numeric_value() {
                Constant::Complex { real, imag } => output.push((real.to_bits(), imag.to_bits())),
                Constant::Tuple(values) | Constant::FrozenSet(values) => {
                    for value in &values {
                        complex_bits(value, output);
                    }
                }
                Constant::Code(code) => {
                    for value in &code.constants {
                        complex_bits(value, output);
                    }
                }
                _ => {}
            }
        }

        let negative_nan = 0xfff8_0000_0000_0000;
        for expression in [
            "(1e309 - 1e309 + 1j) / (1 + 2j)",
            "(1e309 - 1e309 + 1j) / (2 + 1j)",
        ] {
            for source in [
                format!("value = {expression}\n"),
                format!("value = ({expression},)\n"),
                format!("value = True and ({expression})\n"),
                format!("def value():\n    return {expression}\n"),
                format!("def value(arg=({expression})):\n    return arg\n"),
                format!("value = lambda arg=({expression}): arg\n"),
            ] {
                let default = compile(&source, "profile.py").unwrap().marshal();
                let mut outputs = Vec::new();
                for (native_build_profile, expected_imag) in [
                    (NativeBuildProfile::Cpython3145ActionsGcc, negative_nan),
                    (
                        NativeBuildProfile::Cpython3145UvClang,
                        0x7ff8_0000_0000_0000,
                    ),
                ] {
                    let options = CompileOptions {
                        native_build_profile,
                        ..CompileOptions::default()
                    };
                    let module = compile_with_options(&source, "profile.py", &options).unwrap();
                    let mut bits = Vec::new();
                    for value in &module.code.constants {
                        complex_bits(value, &mut bits);
                    }
                    assert_eq!(bits, vec![(negative_nan, expected_imag)], "{source}");
                    outputs.push(module.marshal());
                }
                assert_ne!(outputs[0], outputs[1], "{source}");
                assert_eq!(compile(&source, "profile.py").unwrap().marshal(), default);
            }
        }
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    fn native_profiles_preserve_mixed_and_power_nan_bits() {
        use super::NativeBuildProfile;
        use crate::code_object::Constant;

        let negative_nan = 0xfff8_0000_0000_0000;
        let positive_nan = 0x7ff8_0000_0000_0000;
        for native_build_profile in [
            NativeBuildProfile::Cpython3145ActionsGcc,
            NativeBuildProfile::Cpython3145UvClang,
        ] {
            let options = CompileOptions {
                native_build_profile,
                ..CompileOptions::default()
            };
            let power_imag = if native_build_profile == NativeBuildProfile::Cpython3145ActionsGcc {
                positive_nan
            } else {
                negative_nan
            };
            for (expression, expected) in [
                (
                    "(-(1e309 - 1e309) + 1j) * (1e309 - 1e309)",
                    (negative_nan, negative_nan),
                ),
                ("(1e309 - 1e309 + 1j) ** 0.5", (negative_nan, negative_nan)),
                ("(1e309j - 1e309j) ** 1j", (negative_nan, power_imag)),
                ("(1e309 - 1e309 + 1j) ** -1", (positive_nan, positive_nan)),
            ] {
                let source = format!("value = {expression}\n");
                let module = compile_with_options(&source, "profile.py", &options).unwrap();
                let Constant::Complex { real, imag } = module
                    .code
                    .constants
                    .last()
                    .unwrap()
                    .clone()
                    .into_numeric_value()
                else {
                    panic!("expected a folded complex value");
                };
                assert_eq!(
                    (real.to_bits(), imag.to_bits()),
                    expected,
                    "{native_build_profile}: {expression}"
                );
            }
        }
    }

    #[test]
    fn byte_decoding_updates_intern_state_without_mutating_callers() {
        let absent = InitialInternState::from_partial_with_absence([] as [&str; 0], |_, _| true);
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                initial_intern_state: absent.clone(),
                ..CompileOptions::default()
            };
            let source = b"# coding: cp1251\npass\n";
            let module = compile_bytes_with_options(source, "module.py", &options).unwrap();
            let path_module =
                compile_bytes_with_path_and_options(source, Path::new("module.py"), &options)
                    .unwrap();
            for result in [module, path_module] {
                assert_eq!(
                    result
                        .initial_intern_state
                        .membership_for(python_version, "encoding_table"),
                    Some(true)
                );
                assert_eq!(
                    result
                        .initial_intern_state
                        .membership_for(python_version, "decoding_table"),
                    Some(true)
                );
                assert_eq!(
                    result
                        .initial_intern_state
                        .membership_for(python_version, "unrelated_absent"),
                    Some(false)
                );
            }
            assert_eq!(
                options
                    .initial_intern_state
                    .membership_for(python_version, "encoding_table"),
                Some(false)
            );
            let text =
                compile_with_options("# coding: cp1251\npass\n", "module.py", &options).unwrap();
            assert_eq!(
                text.initial_intern_state
                    .membership_for(python_version, "encoding_table"),
                Some(false)
            );
            let fast =
                compile_bytes_with_options(b"# coding: ascii\npass\n", "module.py", &options)
                    .unwrap();
            assert_eq!(
                fast.initial_intern_state
                    .membership_for(python_version, "ascii_encode"),
                Some(false)
            );
            let alias =
                compile_bytes_with_options(b"# coding: cp367\npass\n", "module.py", &options)
                    .unwrap();
            assert_eq!(
                alias
                    .initial_intern_state
                    .membership_for(python_version, "ascii_encode"),
                Some(true)
            );
            assert_eq!(
                alias
                    .initial_intern_state
                    .membership_for(python_version, "cp367"),
                Some(true)
            );
            let multibyte =
                compile_bytes_with_options(b"# coding: euc-jp\npass\n", "module.py", &options)
                    .unwrap();
            for name in ["readlines", "writelines", "stream"] {
                assert_eq!(
                    multibyte
                        .initial_intern_state
                        .membership_for(python_version, name),
                    Some(true),
                    "{python_version}, {name}"
                );
            }
            let custom = CompileOptions {
                python_version,
                initial_intern_state: absent.without_standard_source_codec_effects(),
                ..CompileOptions::default()
            };
            let module = compile_bytes_with_options(source, "module.py", &custom).unwrap();
            assert_eq!(
                module
                    .initial_intern_state
                    .membership_for(python_version, "encoding_table"),
                Some(false)
            );
            let partial = CompileOptions {
                python_version,
                initial_intern_state: InitialInternState::from_partial([] as [&str; 0]),
                ..CompileOptions::default()
            };
            let module = compile_bytes_with_options(source, "module.py", &partial).unwrap();
            assert_eq!(
                module
                    .initial_intern_state
                    .membership_for(python_version, "encoding_table"),
                Some(true)
            );
            assert_eq!(
                module
                    .initial_intern_state
                    .membership_for(python_version, "unknown_value"),
                None
            );
        }
    }

    #[test]
    fn source_codecs_resolve_retained_qualified_names_for_modern_targets() {
        use crate::CompileError;

        let source = "# coding: cp1251\nclass Codec:\n    class encode: pass\n";
        let state = InitialInternState::from_partial_with_absence([] as [&str; 0], |_, value| {
            value != "Codec.encode"
        });
        for python_version in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                initial_intern_state: state.clone(),
                ..CompileOptions::default()
            };
            assert_eq!(
                compile_with_options(source, "codec.py", &options).unwrap_err(),
                CompileError::UnresolvedInternState {
                    value: "Codec.encode".into()
                }
            );
            let module =
                compile_bytes_with_options(source.as_bytes(), "codec.py", &options).unwrap();
            assert_eq!(
                module
                    .initial_intern_state
                    .membership_for(python_version, "Codec.encode"),
                Some(true)
            );
        }
        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            initial_intern_state: state,
            ..CompileOptions::default()
        };
        let module = compile_bytes_with_options(source.as_bytes(), "codec.py", &options).unwrap();
        assert_eq!(
            module
                .initial_intern_state
                .membership_for(PythonVersion::Py312, "Codec.encode"),
            None
        );
    }

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
