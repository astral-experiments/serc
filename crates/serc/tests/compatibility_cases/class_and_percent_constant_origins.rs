use crate::common;

#[test]
fn class_qualified_names_preserve_constant_admission_order() {
    for future in ["", "from __future__ import annotations\n"] {
        for prefix in [
            "saved = ('Outer.Inner', 'Outer.Other')\n",
            "if condition:\n    saved = ('Outer.Inner', 'Outer.Other')\n",
            "def earlier():\n    return ('Outer.Inner', 'Outer.Other')\n",
        ] {
            let source = format!(
                "{future}{prefix}class Outer:\n    class Inner:\n        value: int\n    class Other:\n        value: int\n"
            );
            common::assert_matches_cpython_marshal(&source, "class_constant_origins.py");
        }
    }
}

#[test]
fn percent_replacement_preserves_fresh_character_identity() {
    for before in ["", "saved = '%'\n", "saved = ('%',)\n"] {
        for after in ["", "later = '%'\n", "later = ('%',)\n"] {
            let source =
                format!("{before}def format_value(value):\n    return '%%%s' % (value,)\n{after}");
            common::assert_matches_cpython_marshal(&source, "percent_constant_origins.py");
        }
    }
    for source in [
        "def format_value(value):\n    return '%' '%%s' % (value,)\n",
        "if False:\n    def discarded(value):\n        return '%%%s' % (value,)\nsaved = '%'\n",
        "def outer(value):\n    if condition:\n        return '%%%s' % (value,)\n    return '%'\n",
        "def format_value(value):\n    return '-%s' % (value,)\n",
        "def format_value(value):\n    return '%%%s' % (value,)\nitems['%':':']\n",
        "items['%':':']\ndef format_value(value):\n    return '%%%s' % (value,)\n",
        "def format_value(value):\n    return '%%%s' % (value,)\nitem in {'%', '-'}\n",
    ] {
        common::assert_matches_cpython_marshal(source, "percent_constant_origins.py");
    }
}

#[test]
fn class_constants_distinguish_earlier_code_qualified_names() {
    for body in [
        "    def Nested():\n        pass\n    class Nested:\n        pass\n",
        "    class Nested:\n        pass\n    def Nested():\n        pass\n",
        "    class Nested:\n        pass\n    class Nested:\n        pass\n",
    ] {
        common::assert_matches_cpython_marshal(
            &format!("class Outer:\n{body}"),
            "earlier_code_qualified_name.py",
        );
    }
}

#[test]
fn class_constant_does_not_alias_an_equal_interned_filename() {
    common::assert_matches_cpython_marshal(
        "class OuterFilename827:\n    class Nested:\n        pass\n",
        "OuterFilename827.Nested",
    );
}
