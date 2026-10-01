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

fn sources() -> Vec<String> {
    let mut sources = Vec::new();
    for loops in 1..=3 {
        for outer_handler in [false, true] {
            for asynchronous in [false, true] {
                for returned in ["[]", "None", "value"] {
                    for cleanup in ["pass", "call", "await"] {
                        let expression = match cleanup {
                            "pass" => "pass",
                            "await" if asynchronous => "await work()",
                            _ => "work()",
                        };
                        let finalizer =
                            format!("try:\n    {expression}\nexcept Exception:\n    pass\n");
                        let mut body = format!("try:\n    return {returned}\n");
                        if outer_handler {
                            body.push_str("except Exception as error:\n    pass\n");
                        }
                        body.push_str("finally:\n");
                        body.push_str(&indent(&finalizer));
                        for index in 0..loops {
                            body = format!("for item{index} in items:\n{}", indent(&body));
                        }
                        body.push_str("return []\n");
                        let scope = if asynchronous { "async " } else { "" };
                        sources.push(format!(
                            "{scope}def sample(items, flag, value):\n{}",
                            indent(&body)
                        ));
                    }
                }
            }
        }
    }
    sources
}

#[test]
fn retained_iterator_return_arrays_keep_native_exit_locations() {
    let sources = sources();
    assert_eq!(sources.len(), 108);
    let directory = tempfile::tempdir().unwrap();
    for (version, variable) in [
        (PythonVersion::Py312, "SERC_TEST_CPYTHON_REFERENCE_312"),
        (PythonVersion::Py313, "SERC_TEST_CPYTHON_REFERENCE_313"),
        (PythonVersion::Py314, "SERC_TEST_CPYTHON_REFERENCE"),
        (PythonVersion::Py315, "SERC_TEST_CPYTHON_REFERENCE_315"),
    ] {
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
