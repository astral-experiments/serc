use crate::common;

#[test]
fn matches_cpython_discarded_private_parameter_assembly_order() {
    for future in ["", "from __future__ import annotations\n"] {
        for parameters in [
            "self, *, __key=1",
            "self, __key: int",
            "self, *, __key: int=1",
            "self, __key",
            "self, *, __key=lambda: '_Container__key'",
            "self, *, __key='_Container__key'",
        ] {
            for member in ["_Container__key", "__key"] {
                for before in [false, true] {
                    for prefix in [false, true] {
                        let consumers = format!(
                            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\n\
                             def second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
                        );
                        let seed = format!(
                            "if False:\n    class Container:\n        def seed({parameters}):\n            pass\n"
                        );
                        let source = if before {
                            seed + &consumers
                        } else {
                            consumers + &seed
                        };
                        let prefix = if prefix {
                            format!("prior = '{member}'\n")
                        } else {
                            String::new()
                        };
                        common::assert_matches_cpython_marshal(
                            &format!("{future}{prefix}{source}"),
                            "discarded_private_parameter_assembly.py",
                        );
                    }
                }
            }
        }
    }
}

fn check_discarded_body(body: &str, parameters: &str) {
    let mut indented = String::new();
    for line in body.lines() {
        indented.push_str("            ");
        indented.push_str(line);
        indented.push('\n');
    }
    for before in [false, true] {
        for prefix in ["", "prior = '_Container__key'\n"] {
            let consumers = "def first(value):\n    return value in {'_Container__key', 'novel_marker'}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {'_Container__key', 'novel_marker'}\n";
            let seed = format!(
                "if False:\n    class Container:\n        def seed({parameters}):\n{indented}"
            );
            let source = if before {
                seed + consumers
            } else {
                consumers.to_string() + &seed
            };
            common::assert_matches_cpython_marshal(
                &format!("{prefix}{source}"),
                "discarded_private_body_assembly.py",
            );
        }
    }
}

#[test]
fn matches_cpython_discarded_private_local_and_name_assembly() {
    for body in [
        "__key = '_Container__key'\n",
        "__key = None\n",
        "self.__key = '_Container__key'\n",
        "def __key():\n    pass\n",
        "'_Container__key'\n__key = None\n",
        "__key = ('_Container__key', 1)\n",
        "return '_Container__key'\n",
    ] {
        for parameters in ["self", "self, *, __key=1", "self, __key: int"] {
            check_discarded_body(body, parameters);
        }
    }
    check_discarded_body("global __key\n__key = '_Container__key'\n", "self");
}

#[test]
fn matches_cpython_discarded_private_closure_assembly() {
    for body in [
        "__key = None\ndef inner():\n    consume(__key)\n    return '_Container__key'\n",
        "__key = None\ndef inner():\n    value = '_Container__key'\n    return __key\n",
        "__key = None\ndef inner():\n    nonlocal __key\n    __key = '_Container__key'\n",
    ] {
        check_discarded_body(body, "self");
    }
}

#[test]
fn matches_cpython_discarded_type_parameter_and_alias_scopes() {
    for declaration in [
        "class Container[T: '_Container__key']:\n    def seed(self, *, __key=1):\n        pass\n",
        "class Container[T = '_Container__key']:\n    def seed(self, *, __key=1):\n        pass\n",
        "class Container:\n    def seed[T: '_Container__key'](self, *, __key=1):\n        pass\n",
        "class Container:\n    def seed[T = '_Container__key'](self, *, __key=1):\n        pass\n",
        "class Container:\n    type Alias = '_Container__key'\n    def seed(self, *, __key=1):\n        pass\n",
        "class Container:\n    type Alias[T] = '_Container__key'\n    def seed(self, *, __key=1):\n        pass\n",
        "class Container:\n    class Inner:\n        value = '_Container__key'\n    def seed(self, *, __key=1):\n        pass\n",
        "class Container:\n    __key: '_Container__key'\n    def seed(self, *, __key=1):\n        pass\n",
        "class Outer:\n    class Container:\n        def seed(self, *, __key=1):\n            pass\n",
    ] {
        let mut seed = "if False:\n".to_string();
        for line in declaration.lines() {
            seed.push_str("    ");
            seed.push_str(line);
            seed.push('\n');
        }
        for future in ["", "from __future__ import annotations\n"] {
            for before in [false, true] {
                for prefix in ["", "prior = '_Container__key'\n"] {
                    let consumers = "def first(value):\n    return value in {'_Container__key', 'novel_marker'}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {'_Container__key', 'novel_marker'}\n";
                    let source = if before {
                        seed.clone() + consumers
                    } else {
                        consumers.to_string() + &seed
                    };
                    common::assert_matches_cpython_marshal(
                        &format!("{future}{prefix}{source}"),
                        "discarded_annotation_scope_assembly.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_generic_class_private_context() {
    for (declaration, member) in [
        (
            "class Outer:\n    class Container[T: __key]:\n        pass\n",
            "_Outer__key",
        ),
        (
            "class Outer:\n    class Container[T = __key]:\n        pass\n",
            "_Outer__key",
        ),
        (
            "class Outer:\n    class Container[T: __key]:\n        pass\n",
            "_Container__key",
        ),
        (
            "class Outer:\n    class Container[T = __key]:\n        pass\n",
            "_Container__key",
        ),
        (
            "class Container:\n    type Alias = __key\n",
            "_Container__key",
        ),
        (
            "class Outer:\n    class __Container[T: '_Outer__Container']:\n        pass\n",
            "_Outer__Container",
        ),
    ] {
        let mut seed = "if False:\n".to_string();
        for line in declaration.lines() {
            seed.push_str("    ");
            seed.push_str(line);
            seed.push('\n');
        }
        for future in ["", "from __future__ import annotations\n"] {
            for before in [false, true] {
                for prefix in [false, true] {
                    let consumers = format!(
                        "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
                    );
                    let source = if before {
                        seed.clone() + &consumers
                    } else {
                        consumers + &seed
                    };
                    let prefix = if prefix {
                        format!("prior = '{member}'\n")
                    } else {
                        String::new()
                    };
                    common::assert_matches_cpython_marshal(
                        &format!("{future}{prefix}{source}"),
                        "discarded_generic_private_context.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_generic_class_mangling_restrictions() {
    for declaration in [
        "class Outer:\n    class Container[__key: __key]:\n        pass\n",
        "class Outer:\n    class Container[T: __key, __key]:\n        pass\n",
        "class Outer:\n    class Container[__key, T: __key]:\n        pass\n",
        "class Outer:\n    class Container[T: (lambda: __key)]:\n        pass\n",
        "class Outer:\n    class Container[T: (lambda __key: __key)]:\n        pass\n",
        "class Outer:\n    class Container[T: (lambda *, __key=1: __key)]:\n        pass\n",
        "class Outer:\n    class Container[T](__key):\n        pass\n",
        "class Outer:\n    class Container[__key](__key):\n        pass\n",
        "class Outer:\n    class Container[__key]:\n        __key = '_Container__key'\n",
        "class Outer:\n    class Container[__key: '_Container__key']:\n        pass\n",
        "class Outer:\n    class Container[T: '_Container__key', __key]:\n        pass\n",
        "class Outer:\n    def seed[__key: '_Outer__key']():\n        pass\n",
        "class Outer:\n    type Alias[__key: '_Outer__key'] = __key\n",
    ] {
        let mut seed = "if False:\n".to_string();
        for line in declaration.lines() {
            seed.push_str("    ");
            seed.push_str(line);
            seed.push('\n');
        }
        for member in ["_Outer__key", "_Container__key", "__key"] {
            for future in ["", "from __future__ import annotations\n"] {
                for before in [false, true] {
                    for prefix in [false, true] {
                        let consumers = format!(
                            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
                        );
                        let source = if before {
                            seed.clone() + &consumers
                        } else {
                            consumers + &seed
                        };
                        let prefix = if prefix {
                            format!("prior = '{member}'\n")
                        } else {
                            String::new()
                        };
                        common::assert_matches_cpython_marshal(
                            &format!("{future}{prefix}{source}"),
                            "discarded_generic_mangling_restrictions.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_discarded_generic_wrapper_assembly() {
    for declaration in [
        "class Outer:\n    def seed[__key]():\n        pass\n",
        "class Outer:\n    type Alias[__key] = int\n",
        "class Outer:\n    def seed[__key]():\n        return '_Outer__key'\n",
        "class Outer:\n    type Alias[__key] = '_Outer__key'\n",
        "class Outer:\n    def seed[__key](arg: '_Outer__key'):\n        pass\n",
        "class Outer:\n    type Alias[__key] = (__key, '_Outer__key')\n",
        "class Outer:\n    class Container[__key]:\n        value = (__key, '_Container__key')\n",
        "class Outer:\n    class Container[__key]:\n        def seed(self):\n            return (__key, '_Container__key')\n",
    ] {
        let mut seed = "if False:\n".to_string();
        for line in declaration.lines() {
            seed.push_str("    ");
            seed.push_str(line);
            seed.push('\n');
        }
        for member in ["_Outer__key", "_Container__key", "__key"] {
            for future in ["", "from __future__ import annotations\n"] {
                for before in [false, true] {
                    for prefix in [false, true] {
                        let consumers = format!(
                            "def first(value):\n    return value in {{'{member}', 'novel_marker'}}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {{'{member}', 'novel_marker'}}\n"
                        );
                        let source = if before {
                            seed.clone() + &consumers
                        } else {
                            consumers + &seed
                        };
                        let prefix = if prefix {
                            format!("prior = '{member}'\n")
                        } else {
                            String::new()
                        };
                        common::assert_matches_cpython_marshal(
                            &format!("{future}{prefix}{source}"),
                            "discarded_generic_wrapper_assembly.py",
                        );
                    }
                }
            }
        }
    }
}
