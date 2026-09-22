use crate::common;

#[test]
fn matches_cpython_nested_class_static_attribute_ownership() {
    for name in ["field", "__field"] {
        for prefix in ["", "from __future__ import annotations\n"] {
            let source = format!(
                "{prefix}class Outer:\n    self.body = 0\n    def method(self):\n        class Inner:\n            self.{name} = 1\n            def method(self):\n                self.inner = 2\n            class Deep:\n                self.deep = 3\n        self.other = 4\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_class_static_attributes.py");
        }
    }
}

#[test]
fn matches_cpython_static_attribute_compiler_unit_boundaries() {
    for body in [
        "items = [0 for self.item in ()]\n",
        "items = (0 for self.item in ())\n",
        "items = lambda: [0 for self.item in ()]\n",
        "class Inner:\n    def method(self, default=[0 for self.item in ()]):\n        self.inner = 1\n",
        "class Inner([0 for self.item in ()]):\n    pass\n",
        "item: (0 for self.item in ())\n",
        "def method(self, item: (0 for self.item in ())):\n    pass\n",
        "def method(self):\n    self.item += [0 for self.other in ()]\n",
        "def method(self):\n    if False:\n        self.item = 1\n",
        "def method(self):\n    return\n    self.item = 1\n",
        "if False:\n    def method(self):\n        self.item = 1\n",
        "def method(self):\n    if False:\n        class Inner:\n            self.item = 1\n",
        "def method(self):\n    if False:\n        self.item += [0 for self.other in ()]\n",
        "def method(self):\n    if False:\n        [0 for self.item in ()].field: int\n",
        "def method(self):\n    if False:\n        value[[0 for self.item in ()]]: int\n",
    ] {
        let source = format!("class Outer:\n{}", common::indent(body));
        common::assert_matches_cpython_marshal(&source, "static_attribute_units.py");
    }
}

#[test]
fn matches_cpython_static_attribute_tuple_cache_for_nested_stores() {
    for body in [
        "def method(self):\n    self.field += [0 for self.other in ()]\n",
        "other: (0 for self.other in ())\n",
    ] {
        for prefix in ["", "prior = ('other',)\n"] {
            let source = format!(
                "{prefix}class Outer:\n{}def first(value):\n    return 'A', value in {{'other', 'x', 'y', 'z'}}\ndef second(value):\n    return 'B', value in {{'other', 'x', 'y', 'z'}}\n",
                common::indent(body),
            );
            common::assert_matches_cpython_marshal(&source, "static_attribute_tuple_cache.py");
        }
    }
}

#[test]
fn matches_cpython_class_declarations_preserve_prior_location() {
    for prefix in ["", "@decorate\n"] {
        for body in [
            "global value\n",
            "global __firstlineno__\n",
            "global first, second\nglobal third\n",
            "\"class documentation\"\nglobal value\n",
            "pass\nglobal value\n",
            "value = 1\nglobal other\n",
        ] {
            let source = format!("{prefix}class C:\n{}", common::indent(body));
            common::assert_matches_cpython_marshal(&source, "class_declaration_location.py");
        }
    }
    for body in [
        "nonlocal value\n",
        "nonlocal __firstlineno__\n",
        "nonlocal value\nglobal other\n",
        "\"class documentation\"\nnonlocal value\n",
        "pass\nnonlocal value\n",
    ] {
        let source = format!(
            "def outer():\n    value = __firstlineno__ = 1\n    class C:\n{}    return C\n",
            common::indent(&common::indent(body))
        );
        common::assert_matches_cpython_marshal(&source, "class_declaration_location.py");
    }
}
