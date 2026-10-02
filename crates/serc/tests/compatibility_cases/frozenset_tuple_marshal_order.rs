use crate::{common, startup_reference};

#[test]
fn matches_cpython_frozenset_tuple_marshal_order() {
    let displays = [
        "{('name', False), ('hyphen-name', False)}",
        "{('name', 'name'), ('name', 'other'), ('name', 'other-name')}",
        "{(300, 300), (300, 301), (301, 300)}",
        "{(1.5, 'value'), (257, 'value'), ('name', 'value')}",
        "{(b'bytes', 2), (b'bytes', 3), (b'other', 2)}",
        "{((300, 'name'), (300, 'name')), ((300, 'name'), (301, 'name'))}",
        "{('hyphen-name', 250), ('other-name', 250), ('other-name', 251)}",
        "{(-257, 2.5), (-258, 2.5), (2.5, -257), (2j, 2.5)}",
    ];
    for display in displays {
        for prefix in [
            "",
            "anchor = 'hyphen-name'\n",
            "anchor = 300\n",
            "anchor = 2.5\n",
        ] {
            let source = format!("{prefix}def f():\n    return {display}\n");
            common::assert_matches_cpython_marshal(&source, "frozenset_tuple_order.py");
        }
    }
}

#[test]
fn matches_cpython_frozenset_scalar_explicit_intern_marshal_order() {
    for (class, parameter) in [
        ("公", "š"),
        ("š", "公"),
        ("name", "公"),
        ("公", "é"),
        ("𐐀", "Ω"),
    ] {
        for members in [
            format!("'{parameter}', '{class}'"),
            format!("'{class}', '{parameter}'"),
            format!("'{parameter}', '{class}', 'Ӝ', 'plain', 'with-hyphen'"),
        ] {
            for prefix in [String::new(), format!("before = '{parameter}'\n")] {
                let source = format!(
                    "{prefix}class {class}:\n    def _thing(self, {parameter}):\n        return {parameter} in {{{members}}}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "frozenset_scalar_explicit_intern_order.py",
                );
            }
        }
    }
    for source in [
        "class 公:\n    def contains(self, value):\n        return value in {'š', '公'}\n",
        "def contains(value):\n    return value in {'š', '公'}\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "frozenset_scalar_explicit_intern_controls.py",
        );
    }
}

#[test]
fn matches_cpython_frozenset_startup_string_identity() {
    for member in ["error", "missing", "other", "timestamp", "u7", "unknown"] {
        let source = format!(
            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}, 1\n\
             def second(value):\n    'second docstring'\n    \
                 return value in {{'{member}', 'novel_marker'}}, 2\n"
        );
        common::assert_matches_cpython_marshal(&source, "frozenset_startup_identity.py");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn matches_cpython_frozenset_native_startup_string_identity() {
    // Repeated equal frozensets expose whether a string had a retained canonical
    // owner before compilation. Cover native modules and their type descriptors.
    for member in [
        "getvalue",
        "getbuffer",
        "_CHUNK_SIZE",
        "is_file",
        "O_RDONLY",
        "struct_rusage",
        "ascii_decode",
        "_unregister_error",
        "__getformat__",
        "get_native_id",
        "_remove_dead_weakref",
        "get_clock_info",
        "get_int_max_str_digits",
        "SIG_DFL",
        "evaluate_bound",
    ] {
        let source = format!(
            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}, 1\n\
             def second(value):\n    'second docstring'\n    \
                 return value in {{'{member}', 'novel_marker'}}, 2\n"
        );
        startup_reference::assert_matches(&source, "startup_interned_names.py");
    }
}

#[test]
fn matches_cpython_frozenset_annotation_string_origins() {
    for future in ["", "from __future__ import annotations\n"] {
        for before_method in [false, true] {
            for assignment in ["", " = ()"] {
                for prefix in [
                    "",
                    "label = 'field_x'\n",
                    "label = ('field_x',)\n",
                    "if False:\n    label = 'field_x'\n",
                    "if False:\n    label = 'field_' + 'x'\n",
                    "def discarded():\n    return None\n    label = 'field_x'\n",
                ] {
                    let declaration = format!("    field_x: int{assignment}\n");
                    let (before, after) = if before_method {
                        (declaration.as_str(), "")
                    } else {
                        ("", declaration.as_str())
                    };
                    let source = format!(
                        "{future}{prefix}class Example:\n{before}    \
                             def first(value):\n        return value in {{'field_x', 'novel_marker'}}\n\
                         {after}class Second:\n    \
                             def second(value):\n        result = value in {{'field_x', 'novel_marker'}}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "frozenset_annotation_origins.py",
                    );
                    common::assert_matches_cpython_marshal(
                        &source.replace("field_x", "sort"),
                        "frozenset_annotation_static_origins.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_explicit_string_origins() {
    for member in ["id", "status", "custom_flag"] {
        for seed in [
            format!("call({member}=1)\n"),
            format!("from package import {member}\n"),
            format!("match subject:\n    case Example({member}=_):\n        matched = True\n"),
        ] {
            for before_functions in [false, true] {
                for prefix in [
                    String::new(),
                    format!("earlier = '{member}'\n"),
                    format!("earlier = ('{member}',)\n"),
                    format!("if False:\n    earlier = '{member}'\n"),
                ] {
                    let (before, after) = if before_functions {
                        (seed.as_str(), "")
                    } else {
                        ("", seed.as_str())
                    };
                    let source = format!(
                        "{prefix}{before}def first(value):\n    return value in {{'{member}', 'novel_marker'}}, 1\n\
                         def second(value):\n    return value in {{'{member}', 'novel_marker'}}, 2\n{after}"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "frozenset_explicit_origins.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_skipped_keyword_string_origins() {
    for seed in [
        "call(custom_flag=1)",
        "from package import custom_flag",
        "call(custom_flag='custom_flag')",
        "call(custom_flag=1, other='custom_flag')",
        "call(*args, custom_flag=1)",
        "call(*args, custom_flag='custom_flag')",
        "call(**kwargs, custom_flag=1)",
    ] {
        for skipped in [false, true] {
            for prefix in ["", "earlier = 'custom_flag'\n"] {
                let seed = if skipped {
                    format!("if False:\n    {seed}\n")
                } else {
                    format!("{seed}\n")
                };
                let source = format!(
                    "{prefix}{seed}def first(value):\n    return value in {{'custom_flag', 'novel_marker'}}, 1\n\
                     def second(value):\n    return value in {{'custom_flag', 'novel_marker'}}, 2\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "frozenset_skipped_keyword_origins.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_nested_keyword_string_origins() {
    for prefix in ["", "earlier = ('first_name', 'custom_flag')\n"] {
        for seed in ["call(custom_flag=1)", "call(other_flag=0, custom_flag=1)"] {
            let source = format!(
                "{prefix}def annotated(other_marker: object):\n    pass\n\
                 def seed():\n    return {seed}\n\
                 def first(value):\n    return value in {{'custom_flag', 'other_marker'}}\n\
                 def second(value):\n    other_marker: object = None\n    \
                     return value in {{'custom_flag', 'other_marker'}}\n"
            );
            common::assert_matches_cpython_marshal(&source, "frozenset_nested_keyword_origins.py");
        }
    }
}

#[test]
fn matches_cpython_frozenset_parameter_string_origins() {
    for parameters in [
        "member_x: object",
        "*, member_x: object = None",
        "*, member_x=None",
        "*member_x: object",
        "**member_x: object",
    ] {
        for prefix in ["", "earlier = ('member_x',)\n"] {
            for seed_before_sets in [false, true] {
                let seed = format!("def seed({parameters}):\n    pass\n");
                let (before, after) = if seed_before_sets {
                    (seed.as_str(), "")
                } else {
                    ("", seed.as_str())
                };
                let source = format!(
                    "{prefix}{before}def first(value):\n    \
                         return value in {{'member_x', 'novel_marker'}}, 1\n\
                     def second(value):\n    'second docstring'\n    \
                         return value in {{'member_x', 'novel_marker'}}, 2\n{after}"
                );
                common::assert_matches_cpython_marshal(&source, "frozenset_parameter_origins.py");
            }
        }
    }
}

#[test]
fn matches_cpython_frozenset_dotted_import_identifiers() {
    for import in [
        "from pkgroot.branch import item",
        "import pkgroot.branch as alias",
        "import pkgroot.branch",
        "from .pkgroot.branch import item",
        "from pkgroot import item",
        "if False:\n    from pkgroot.branch import item",
    ] {
        for prefix in ["", "earlier = 'pkgroot'\n"] {
            let source = format!(
                "{prefix}def first(value):\n    return value in {{'pkgroot', 'novel_marker'}}, 1\n\
                 def second(value):\n    'second docstring'\n    \
                     return value in {{'pkgroot', 'novel_marker'}}, 2\n\
                 {import}\n"
            );
            common::assert_matches_cpython_marshal(&source, "frozenset_dotted_imports.py");
        }
    }
}
