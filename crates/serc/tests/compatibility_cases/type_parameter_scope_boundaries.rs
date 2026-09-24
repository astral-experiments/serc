use crate::common;

#[test]
fn alias_parameters_do_not_capture_enclosing_bindings() {
    for source in [
        r"def outer():
    type Alias[T] = lambda: T
    T, = Alias.__type_params__
    return T, Alias
",
        r"def outer():
    def generic[T, *Ts, **P]():
        type Alias[T2, *Ts2, **P2] = tuple[T, *Ts, P, T2, *Ts2, P2]
        return Alias
    T2, Ts2, P2 = generic().__type_params__
    return T2, Ts2, P2
",
        r"def outer(T, value):
    class C[T]:
        callback = lambda: (T, value)
    return C, T
",
        r"def outer(T):
    @decorate(lambda: T)
    class C[T]:
        def method(self):
            return T
    return C, T
",
        r"def outer(__T):
    class C[__T]:
        callback = lambda: __T
    return C, __T
",
    ] {
        common::assert_matches_cpython_marshal(source, "type_parameter_scope_boundary.py");
    }
}

#[test]
fn alias_comprehension_targets_do_not_claim_enclosing_local_slots() {
    for source in [
        r"def outer(T, values):
    type Alias = [T for item in values]
    return Alias, T
",
        r"def outer():
    type Alias[T: [lambda: T for T in (T, [1])[1]]] = [lambda: T for T in T.__name__]
    func, = Alias.__value__
    T, = Alias.__type_params__
    return func, T
",
    ] {
        common::assert_matches_cpython_marshal(source, "alias_comprehension_local_slots.py");
    }
}

#[test]
fn nested_alias_expressions_keep_synthetic_class_cells() {
    for declaration in ["", "    __class__ = int\n", "    global __class__\n"] {
        for alias in [
            "type Alias[T] = lambda: __class__",
            "type Alias[T] = lambda: super()",
            "type Alias[T: lambda: __class__] = T",
            "type Alias[__class__] = lambda: __class__",
        ] {
            let source = format!("class C:\n{declaration}    {alias}\n");
            common::assert_matches_cpython_marshal(&source, "alias_synthetic_class_cell.py");
        }
    }
}

#[test]
fn nested_generic_class_parameters_shadow_other_private_names() {
    for (binding_index, binding) in [
        "__T = 'different private name'",
        "global __T",
        "_Inner__T = 'same private name'",
        "global _Inner__T",
    ]
    .into_iter()
    .enumerate()
    {
        for (parameter_index, parameters) in [
            "__T: __T",
            "__T, U: __T",
            "__T: (__T, int)",
            "__T: lambda: __T",
            "__T: [__T for _ in (0,)]",
            "__T = __T",
            "__T, U = __T",
        ]
        .into_iter()
        .enumerate()
        {
            let source =
                format!("class Outer:\n    {binding}\n    class Inner[{parameters}]: pass\n");
            let filename =
                format!("nested_generic_class_private_{binding_index}_{parameter_index}.py");
            common::assert_matches_cpython_marshal(&source, &filename);
        }
    }

    common::assert_executes(
        r"class Outer:
    __T = 'different private name'
    class Inner[__T: __T]: pass
    class Pair[__T, U: __T]: pass
    class Nested[__T: lambda: __T]: pass
    class Default[__T = __T]: pass
    _Spelled__T = 'same private name'
    class Spelled[__T: __T]: pass
    T = 'public name'
    class Public[T: T]: pass
    class Outer[__T: __T]: pass

print(Outer.Inner.__type_params__[0].__bound__ is Outer.Inner.__type_params__[0])
print(Outer.Pair.__type_params__[1].__bound__ is Outer.Pair.__type_params__[0])
print(Outer.Nested.__type_params__[0].__bound__() is Outer.Nested.__type_params__[0])
print(Outer.Default.__type_params__[0].__default__ is Outer.Default.__type_params__[0])
print(Outer.Spelled.__type_params__[0].__bound__)
print(Outer.Public.__type_params__[0].__bound__)
print(Outer.Outer.__type_params__[0].__bound__)
",
        "True\nTrue\nTrue\nTrue\nsame private name\npublic name\ndifferent private name\n",
    );
}
