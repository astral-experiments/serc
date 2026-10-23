use serc::{CompileError, CompileOptions, PythonVersion, compile_bytes_with_options};

#[test]
fn normalized_singleton_identifiers_follow_the_target_parser() {
    for (source, name) in [
        ("Ｔrue = 0", "True"),
        ("obj.Ｆａｌｓｅ", "False"),
        ("from pkg.Ｎｏｎｅ import value", "None"),
        ("f'{Truℯ}'", "True"),
        ("if False:\n    x = Ｔrue", "True"),
    ] {
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version,
                ..CompileOptions::default()
            };
            let result = compile_bytes_with_options(source.as_bytes(), "case.py", &options);
            if python_version == PythonVersion::Py312 {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert_eq!(error, CompileError::InvalidIdentifier(name.to_owned()));
                assert_eq!(
                    error.to_string(),
                    format!("identifier field can't represent '{name}' constant")
                );
            }
        }
    }
    for source in [
        "x = True, False, None",
        "ｘ = 'Ｔrue' # Ｎｏｎｅ",
        "ＴrueValue = 1",
        "ｉｆ = 0",
    ] {
        serc::compile(source, "case.py").unwrap();
    }
}

#[test]
fn normalized_names_follow_parser_error_order() {
    let options = CompileOptions {
        python_version: PythonVersion::Py313,
        ..CompileOptions::default()
    };
    for source in [
        "Ｔrue\nif :",
        "1 = Ｔrue",
        "1 += Ｔrue",
        "del 0, Ｔrue",
        "1: Ｔrue",
        "t\"{Ｎｏｎｅ}\" + Ｔrue",
        "Ｔrue\nt\"\"",
    ] {
        assert_eq!(
            compile_bytes_with_options(source.as_bytes(), "case.py", &options).unwrap_err(),
            CompileError::InvalidIdentifier("True".to_owned()),
            "{source}"
        );
    }
    for source in [
        "if :\n    Ｔrue",
        "1 = 1; Ｔrue",
        "1 = 1\nＴrue",
        "t\"{Ｔrue}\"",
        "t\"\"\nＴrue",
    ] {
        assert!(
            matches!(
                compile_bytes_with_options(source.as_bytes(), "case.py", &options),
                Err(CompileError::InvalidSyntax { .. } | CompileError::Parse(_))
            ),
            "{source}"
        );
    }
}
