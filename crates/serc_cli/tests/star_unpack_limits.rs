mod common;

use serc::CompileOptions;

#[test]
fn oversized_star_unpacking_matches_cpython() {
    let target = format!("{}*rest", "a,".repeat(256));
    let pattern = format!("{}*rest", "_,".repeat(256));
    for version in common::PYTHON_VERSIONS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for source in [
            format!("{target} = seq\n"),
            format!("for [{target}] in seq:\n    pass\n"),
            format!("match seq:\n    case [{pattern}]:\n        pass\n"),
        ] {
            let diagnostic = common::diagnostics::syntax_diagnostic_with_bytes(version, &source);
            diagnostic.assert_syntax_error(&source, &options, diagnostic.byte_range(&source));
        }
        for source in [
            format!("{}*rest = seq\n", "a,".repeat(255)),
            format!("*rest,{} = seq\n", "a,".repeat(256)),
            format!("match seq:\n    case [{}*_]: pass\n", "_,".repeat(256)),
        ] {
            serc::compile_with_options(&source, "case.py", &options).unwrap();
        }
    }
}
