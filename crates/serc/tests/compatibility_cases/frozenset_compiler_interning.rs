use crate::common;

#[test]
fn matches_cpython_frozenset_compiler_initialized_identifiers() {
    for member in [
        "left",
        "right",
        "body",
        "orelse",
        "comparators",
        "kw_defaults",
    ] {
        let source = format!(
            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
             def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
        );
        common::assert_matches_cpython_startup_marshal(&source, "frozenset_compiler_interning.py");
    }
}

#[test]
fn matches_cpython_frozenset_isolated_startup_identifiers() {
    for member in [
        "None",
        "candidate",
        "min",
        "nan",
        "new",
        "output",
        "remove",
        "root",
        "sum",
        "MISSING",
        "cache",
        "cached",
        "eval",
        "map",
        "pause",
        "python",
        "quiet",
        "train",
        "pretty",
        "replay",
        "completed",
        "generated",
        "both",
        "none",
        "rust",
        "array",
        "structured",
        "date",
        "link",
        "old_value",
        "package",
        "run",
        "system",
        "test",
        "user",
        "var",
        "warn",
        "left_extra",
        "desc",
        "failed",
        "full",
        "high",
        "partial",
    ] {
        let source = format!(
            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
             def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
        );
        common::assert_matches_cpython_startup_marshal(&source, "frozenset_startup_identifiers.py");
    }
}

#[test]
fn matches_cpython_startup_strings_with_prior_constants_and_private_names() {
    for member in [
        "None",
        "candidate",
        "min",
        "nan",
        "new",
        "output",
        "remove",
        "root",
        "sum",
    ] {
        for seed in 0..3 {
            let prefix = match seed {
                0 => format!("prior = '{member}'\n"),
                1 => format!("if False:\n    prior = '{member}'\n"),
                _ => "class Container:\n    def method(self, __key):\n        return __key\n"
                    .to_string(),
            };
            let source = format!(
                "{prefix}def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
                 def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
            );
            common::assert_matches_cpython_startup_marshal(&source, "frozenset_startup_origins.py");
        }
    }
}
