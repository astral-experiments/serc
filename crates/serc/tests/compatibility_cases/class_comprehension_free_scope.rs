use crate::common::{assert_executes, assert_matches_cpython_marshal};

#[test]
fn class_generators_capture_the_class_cell() {
    for expression in [
        "(__class__ for item in (1,))",
        "(super for item in (1,))",
        "((lambda: __class__) for item in (1,))",
        "((__class__ for inner in (1,)) for item in (1,))",
        "(item for item in (1,) if __class__)",
        "(item for item in (1,) for other in (__class__,))",
        "(item for item in (__class__,))",
        "(__class__ for __class__ in (1,))",
    ] {
        for source in [
            format!("class C:\n    values = {expression}\n"),
            format!(
                "def outer():\n    __class__ = 'outer'\n    class C:\n        values = {expression}\n    return C\n"
            ),
        ] {
            assert_matches_cpython_marshal(&source, "class_generator_free_scope.py");
        }
    }

    assert_executes(
        r"class C:
    values = (__class__ for item in (1,))
    nested = ((__class__ for inner in (1,)) for item in (1,))
    explicit_target = (__class__ for __class__ in (23,))
print(next(C.values) is C, next(next(C.nested)) is C, next(C.explicit_target))
",
        "True True 23\n",
    );
}

#[test]
fn class_inlined_comprehensions_keep_dunder_class_global() {
    for expression in [
        "[__class__ for item in (1,)]",
        "{__class__ for item in (1,)}",
        "{item: __class__ for item in (1,)}",
        "[[__class__ for inner in (1,)] for item in (1,)]",
        "[item for item in (1,) if __class__]",
        "[item for item in (1,) for other in (__class__,)]",
        "[item for item in __class__]",
        "[__class__ for __class__ in (1,)]",
    ] {
        for before in ["", "        direct = __class__\n"] {
            let source = format!(
                "__class__ = 'global'\ndef outer():\n    __class__ = 'outer'\n    class C:\n{before}        values = {expression}\n    return C\n"
            );
            assert_matches_cpython_marshal(&source, "class_inline_dunder_class.py");
        }
    }

    assert_executes(
        r"__class__ = 'global'
def outer():
    __class__ = ('outer',)
    class C:
        listed = [__class__ for item in (1,)]
        grouped = {__class__ for item in (1,)}
        mapped = {item: __class__ for item in (1,)}
        nested = [[__class__ for inner in (1,)] for item in (1,)]
    class D:
        first_iterable = [item for item in __class__]
    return C, D
C, D = outer()
print(C.listed, sorted(C.grouped), C.mapped, C.nested, D.first_iterable)
",
        "['global'] ['global'] {1: 'global'} [['global']] ['outer']\n",
    );
}

#[test]
fn class_inlined_comprehensions_capture_shadowed_enclosing_names() {
    for expression in [
        "[value for item in (1,)]",
        "{value for item in (1,)}",
        "{item: value for item in (1,)}",
        "[[value for inner in (1,)] for item in (1,)]",
        "[item for item in (1,) if value]",
        "[item for item in (1,) for other in (value,)]",
        "[item for item in value]",
        "[value for value in (1,)]",
    ] {
        for binding in [
            "        value = 'class'\n",
            "        value: str\n",
            "        del value\n",
            "        global value\n",
            "        nonlocal value\n",
        ] {
            let source = format!(
                "value = 'global'\ndef outer():\n    value = 'outer'\n    class C:\n{binding}        values = {expression}\n    return C\n"
            );
            assert_matches_cpython_marshal(&source, "class_inline_shadowed_free.py");
        }
    }

    assert_executes(
        r"value = 'global'
def outer():
    value = 'outer'
    class C:
        value = ('class',)
        listed = [value for item in (1,)]
        grouped = {value for item in (1,)}
        mapped = {item: value for item in (1,)}
        nested = [[value for inner in (1,)] for item in (1,)]
        first_iterable = [item for item in value]
    class D:
        global value
        direct = value
        listed = [value for item in (1,)]
    class E:
        nonlocal value
        direct = value
        listed = [value for item in (1,)]
    return C, D, E
C, D, E = outer()
print(C.listed, sorted(C.grouped), C.mapped, C.nested, C.first_iterable)
print(D.direct, D.listed, E.direct, E.listed)
",
        "['outer'] ['outer'] {1: 'outer'} [['outer']] ['class']\nglobal ['outer'] outer ['outer']\n",
    );
}
