use serc::{
    CompileError, CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options,
};

#[test]
fn enabled_targets_compile_and_disabled_targets_are_rejected() {
    for version in [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ] {
        let expected_enabled =
            version == PythonVersion::Py314 || cfg!(feature = "all-python-versions");
        assert_eq!(version.is_enabled(), expected_enabled);

        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for result in [
            compile_with_options("pass\n", "test.py", &options),
            compile_bytes_with_options(b"pass\n", "test.py", &options),
        ] {
            if expected_enabled {
                assert_eq!(result.unwrap().python_version(), version);
            } else {
                assert!(matches!(
                    result,
                    Err(CompileError::Unsupported(message)) if message.contains("all-python-versions")
                ));
            }
        }
    }
}
