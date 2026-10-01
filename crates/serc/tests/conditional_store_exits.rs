use std::process::Command;

use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile_bytes_with_options,
    compile_with_options,
};

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn sources(located_name_pass: bool) -> Vec<String> {
    let mut sources = Vec::new();
    for scope in ["plain", "async", "generator"] {
        for otherwise in ["none", "pass", "assert", "assignment"] {
            for suffix in ["none", "assert", "pass", "two asserts"] {
                for target in ["value", "obj.value"] {
                    if (suffix == "pass" && target == "value") != located_name_pass {
                        continue;
                    }
                    let suffix = match suffix {
                        "none" => "",
                        "assert" => "assert other\n",
                        "pass" => "pass\n",
                        "two asserts" => "assert other\nassert flag\n",
                        _ => unreachable!(),
                    };
                    let body = format!("{target} = 10 if other else 20\n{suffix}");
                    let mut statement = format!("if flag:\n{}", indent(&body));
                    if otherwise != "none" {
                        let otherwise = match otherwise {
                            "pass" => "pass\n",
                            "assert" => "assert flag\n",
                            "assignment" => "value = 30\n",
                            _ => unreachable!(),
                        };
                        statement.push_str("else:\n");
                        statement.push_str(&indent(otherwise));
                    }
                    if scope == "generator" {
                        statement = format!("yield 1\n{statement}");
                    }
                    let prefix = if scope == "async" { "async " } else { "" };
                    sources.push(format!(
                        "{prefix}def sample(flag, other, obj):\n{}",
                        indent(&statement)
                    ));
                }
            }
        }
    }
    sources
}

#[test]
fn elided_assertions_keep_original_conditional_store_exit_copying() {
    let sources = sources(false);
    assert_eq!(sources.len(), 84);
    assert_matches_references(&sources, &REFERENCES);
}

#[test]
fn located_pass_keeps_legacy_conditional_store_boundary() {
    let sources = sources(true);
    assert_eq!(sources.len(), 12);
    assert_matches_references(&sources, &REFERENCES[..2]);
}

const REFERENCES: [(PythonVersion, &str); 4] = [
    (PythonVersion::Py312, "SERC_TEST_CPYTHON_REFERENCE_312"),
    (PythonVersion::Py313, "SERC_TEST_CPYTHON_REFERENCE_313"),
    (PythonVersion::Py314, "SERC_TEST_CPYTHON_REFERENCE"),
    (PythonVersion::Py315, "SERC_TEST_CPYTHON_REFERENCE_315"),
];

fn assert_matches_references(sources: &[String], references: &[(PythonVersion, &str)]) {
    let directory = tempfile::tempdir().unwrap();
    for &(version, variable) in references {
        let Some(reference) = std::env::var_os(variable) else {
            continue;
        };
        let command = || {
            let mut command = Command::new(&reference);
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("LANG", "C.UTF-8");
            command
        };
        let identity = command().arg("--identity").output().unwrap();
        assert!(identity.status.success(), "{version:?}: {identity:?}");
        let actual: serde_json::Value = serde_json::from_slice(&identity.stdout).unwrap();
        let (release_level, release_serial) = if version == PythonVersion::Py315 {
            ("candidate", 2)
        } else {
            ("final", 0)
        };
        assert_eq!(
            actual,
            serde_json::json!({
                "protocol": "minimal-v2", "implementation": "cpython",
                "version": version.target().version_string(), "release_level": release_level,
                "release_serial": release_serial, "magic": version.target().magic_hex(),
            })
        );
        for (index, source) in sources.iter().enumerate() {
            let path = directory.path().join(format!("case{index}.py"));
            std::fs::write(&path, source).unwrap();
            let filename = path.to_str().unwrap();
            for optimization_level in [
                OptimizationLevel::Zero,
                OptimizationLevel::One,
                OptimizationLevel::Two,
            ] {
                let native = command()
                    .args(["--optimize", &optimization_level.to_string(), filename])
                    .output()
                    .unwrap();
                assert!(
                    native.status.success(),
                    "{version:?} {optimization_level:?}: {native:?}"
                );
                let options = CompileOptions {
                    python_version: version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                for ours in [
                    compile_with_options(source, filename, &options).unwrap(),
                    compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
                ] {
                    assert_eq!(
                        ours.marshal(),
                        native.stdout,
                        "{version:?} {optimization_level:?}:\n{source}"
                    );
                }
            }
        }
    }
}
