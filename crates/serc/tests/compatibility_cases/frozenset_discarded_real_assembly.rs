use crate::common;

const CONSUMERS: &str = "def first(value):\n    return value in {'_Container__key', 'novel_marker'}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {'_Container__key', 'novel_marker'}\n";

fn indent(body: &str) -> String {
    let mut indented = String::new();
    for line in body.lines() {
        indented.push_str("    ");
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}

fn string_tuple(names: &[&str]) -> String {
    let names = names
        .iter()
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("({names},)")
}

fn check(seed: &str, before: bool, prior: bool, filename: &str) {
    let seed = format!("if False:\n{}", indent(seed));
    let source = if before {
        format!("{seed}{CONSUMERS}")
    } else {
        format!("{CONSUMERS}{seed}")
    };
    let prior = if prior {
        "prior = '_Container__key'\n"
    } else {
        ""
    };
    common::assert_matches_cpython_marshal(&format!("{prior}{source}"), filename);
}

#[test]
fn matches_cpython_discarded_class_names_tuple_cache() {
    // Metadata tuples include the synthetic prefix and suffix in their cache key.
    let prefix = [
        "__name__",
        "__module__",
        "__qualname__",
        "__firstlineno__",
        "__type_params__",
    ];
    let cases: [(&str, &[&str], &[&str]); 6] = [
        (
            "__key = 1\n",
            &["_Container__key"],
            &["_Container__key", "__static_attributes__"],
        ),
        (
            "__key = value\n",
            &["value", "_Container__key"],
            &["value", "_Container__key", "__static_attributes__"],
        ),
        (
            "__key: int = 1\n",
            &["_Container__key"],
            &[
                "_Container__key",
                "__annotate_func__",
                "__static_attributes__",
                "__classdictcell__",
            ],
        ),
        (
            "def __key(self):\n    pass\n",
            &["_Container__key"],
            &[
                "_Container__key",
                "__static_attributes__",
                "__classdictcell__",
            ],
        ),
        (
            "__key = lambda: 1\n",
            &["_Container__key"],
            &["_Container__key", "__static_attributes__"],
        ),
        (
            "global __key\n__key = 1\n",
            &["_Container__key"],
            &["_Container__key", "__static_attributes__"],
        ),
    ];
    for (body, authored, suffix) in cases {
        let full = prefix.iter().chain(suffix).copied().collect::<Vec<_>>();
        let reversed = full.iter().rev().copied().collect::<Vec<_>>();
        for names in [full.as_slice(), authored, reversed.as_slice()] {
            let seed = format!(
                "class Container[T: {} * 0]:\n{}",
                string_tuple(names),
                indent(body),
            );
            for before in [false, true] {
                for prior in [false, true] {
                    check(&seed, before, prior, "discarded_class_names_tuple.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_constants_tuple_assembly_order() {
    for seed in [
        "class Container:\n    def a():\n        return '_Container__key'\n    def b():\n        return __key\n",
        "class Container:\n    def a():\n        '_Container__key'\n        return '_Container__key'\n    def b():\n        return __key\n",
        "class Container:\n    a = lambda: '_Container__key'\n    def b():\n        return __key\n",
        "class Container:\n    def a():\n        return ('_Container__key', 'novel_marker')\n    def b():\n        return __key\n",
        "class Container:\n    def a():\n        return __key\n    def b():\n        return '_Container__key'\n",
        "class Container:\n    def a():\n        return '_Container__key'\n    def b(__key):\n        pass\n",
    ] {
        for before in [false, true] {
            for prior in [false, true] {
                check(seed, before, prior, "discarded_constants_tuple.py");
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_class_annotation_assembly() {
    for parameters in ["", "[T]"] {
        let seed = format!("class Container{parameters}:\n    __key: int = 1\n");
        for prior in [false, true] {
            check(&seed, true, prior, "discarded_class_annotation_assembly.py");
        }
    }
}

#[test]
fn matches_cpython_discarded_function_names_tuple_cache() {
    for names in [
        &["obj", "_Container__key"][..],
        &["_Container__key", "obj"][..],
        &["_Container__key"][..],
    ] {
        for returned in ["obj.__key", "__key"] {
            let seed = format!(
                "class Container[T: {} * 0]:\n    def seed():\n        return {returned}\n",
                string_tuple(names),
            );
            check(&seed, true, false, "discarded_function_names_tuple.py");
        }
    }
}
