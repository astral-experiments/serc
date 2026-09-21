use crate::common;
use crate::common::indent;

fn consumers(member: &str) -> String {
    format!(
        "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
         def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
    )
}

#[test]
fn matches_cpython_frozenset_mangled_parameter_origins() {
    for parameters in ["self, __key: int", "self, *, __key=1", "self, **__key: int"] {
        for member in ["__key", "_Container__key"] {
            for before in [false, true] {
                for future in ["", "from __future__ import annotations\n"] {
                    let seed = format!("def seed({parameters}):\n    pass\n");
                    let pair = consumers(member);
                    let body = if before { seed + &pair } else { pair + &seed };
                    let source = format!("{future}class Container:\n{}", indent(&body));
                    common::assert_matches_cpython_marshal(&source, "mangled_parameter_origin.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_unannotated_private_parameters() {
    for parameters in ["self, __key", "self, *__key", "self, **__key"] {
        for prefix in ["", "prior = '_Container__key'\n"] {
            let body =
                consumers("_Container__key") + &format!("def seed({parameters}):\n    pass\n");
            let source = format!("{prefix}class Container:\n{}", indent(&body));
            common::assert_matches_cpython_marshal(&source, "private_parameter_origin.py");
        }
    }
}

#[test]
fn matches_cpython_frozenset_explicit_mangled_identifiers() {
    for raw in [
        "_Container__key = 1\n",
        "def declaration(_Container__key):\n    pass\n",
        "def declaration():\n    global _Container__key\n",
        "try:\n    pass\nexcept Exception as _Container__key:\n    pass\n",
        "match value:\n    case _Container__key:\n        pass\n",
        "match value:\n    case [*_Container__key]:\n        pass\n",
        "match value:\n    case {**_Container__key}:\n        pass\n",
        "def typed[_Container__key]():\n    pass\n",
        "def typed[**_Container__key]():\n    pass\n",
        "def typed[*_Container__key]():\n    pass\n",
        "from package._Container__key import value\n",
    ] {
        for prefix in ["", "prior = '_Container__key'\n"] {
            let body = "def seed(self, __key: int):\n    pass\n".to_string()
                + &consumers("_Container__key");
            let source = format!("{prefix}{raw}class Container:\n{}", indent(&body));
            common::assert_matches_cpython_marshal(&source, "explicit_mangled_identifier.py");
        }
    }
}

#[test]
fn matches_cpython_frozenset_skipped_definition_origins() {
    for seed in [
        "def seed(key: int):\n    pass\n",
        "def seed(*, key=1):\n    pass\n",
        "seed = lambda *, key=1: None\n",
    ] {
        for wrapper in [
            "if False:\n",
            "while False:\n",
            "def outer():\n    return\n",
            "if True:\n    pass\nelse:\n",
        ] {
            for future in ["", "from __future__ import annotations\n"] {
                let source = format!("{future}{wrapper}{}{}", indent(seed), consumers("key"));
                common::assert_matches_cpython_marshal(&source, "skipped_definition_origin.py");
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_nested_skipped_definition_origins() {
    for seed in [
        "def seed(key: int):\n    pass\n",
        "def seed(*, key=1):\n    pass\n",
        "seed = lambda *, key=1: None\n",
    ] {
        for scope in ["def outer():\n", "class Outer:\n"] {
            for prefix in ["", "prior = 'key'\n"] {
                let nested = scope.to_string() + &indent(seed);
                let source = format!("{prefix}if False:\n{}{}", indent(&nested), consumers("key"));
                common::assert_matches_cpython_marshal(&source, "nested_definition_origin.py");
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_definition_name_constants() {
    for seed in [
        "class key:\n    pass\n",
        "def typed[key]():\n    pass\n",
        "type key = int\n",
        "type key[T] = T\n",
    ] {
        for prefix in ["", "prior = 'key'\n"] {
            let source = format!("{prefix}{seed}{}", consumers("key"));
            common::assert_matches_cpython_marshal(&source, "definition_name_origin.py");
        }
    }
}
