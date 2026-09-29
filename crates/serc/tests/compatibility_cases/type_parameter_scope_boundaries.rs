use crate::common;

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
fn class_type_alias_nested_scopes_capture_shadowed_enclosing_names() {
    for alias in [
        "type Alias = lambda: x",
        "type Alias = lambda: lambda: x",
        "type Alias = (x, lambda: x)",
        "type Alias = (x for _ in (0,))",
        "type Alias[T] = lambda: (x, T)",
        "type Alias[T: (lambda: x)] = T",
        "type Alias[T = (lambda: x)] = T",
    ] {
        for declaration in ["x = 'class'", "global x"] {
            let source = format!(
                "x = 'global'\ndef outer(x):\n    class C:\n        {declaration}\n        {alias}\n    return C\n"
            );
            common::assert_matches_cpython_marshal(&source, "class_alias_nested_capture.py");
        }
    }

    for alias in [
        "type Alias = x",
        "type Alias = lambda value=x: value",
        "type Alias[x] = lambda: x",
        "type Alias[x: (lambda: x)] = x",
        "type Alias[x = (lambda: x)] = x",
    ] {
        let source = format!(
            "def outer(x):\n    class C:\n        x = 'class'\n        {alias}\n    return C\n"
        );
        common::assert_matches_cpython_marshal(&source, "class_alias_scope_control.py");
    }

    for alias in [
        "type Alias = lambda: __x",
        "type Alias[T] = lambda: __x",
        "type Alias[T: (lambda: __x)] = T",
        "type Alias[T = (lambda: __x)] = T",
    ] {
        let source = format!(
            "def outer(_C__x):\n    class C:\n        __x = 'class'\n        {alias}\n    return C\n"
        );
        common::assert_matches_cpython_marshal(&source, "class_alias_private_capture.py");
    }
}

#[test]
fn executes_class_type_alias_nested_scopes_with_class_and_enclosing_bindings() {
    let source = "x = 'global'\ndef outer(x):\n    class Bound:\n        x = 'class'\n        type Direct = x\n        type DefaultOnly = lambda value=x: value\n        type Both = (x, lambda: x)\n        type Generic[T] = lambda: (x, T)\n        type Bound[T: (lambda: x)] = T\n        type Default[T = (lambda: x)] = T\n        type Own[x] = lambda: x\n    class Global:\n        global x\n        type Both = (x, lambda: x)\n    return Bound, Global\nBound, Global = outer('enclosing')\nprint(Bound.Direct.__value__, Bound.DefaultOnly.__value__())\nfor cls in (Bound, Global):\n    direct, nested = cls.Both.__value__\n    print(direct, nested())\nvalue, parameter = Bound.Generic.__value__()\nprint(value, parameter is Bound.Generic.__type_params__[0])\nprint(Bound.Bound.__type_params__[0].__bound__())\nprint(Bound.Default.__type_params__[0].__default__())\nprint(Bound.Own.__value__() is Bound.Own.__type_params__[0])\n";
    common::assert_executes(
        source,
        "class class\nclass enclosing\nglobal enclosing\nenclosing True\nenclosing\nenclosing\nTrue\n",
    );
}

#[test]
fn python_312_still_rejects_lambdas_in_class_type_alias_scopes() {
    use serc::{CompileOptions, PythonVersion, compile_with_options};

    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for alias in [
        "type Alias = lambda: x",
        "type Alias[T] = lambda: x",
        "type Alias[T: (lambda: x)] = T",
        "type Alias = (lambda: x, (value := x))",
        "type Alias[T: (lambda: x), T] = int",
    ] {
        let source = format!(
            "def outer(x):\n    class C:\n        x = 'class'\n        {alias}\n    return C\n"
        );
        let error =
            compile_with_options(&source, "class_alias_nested_capture.py", &options).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Cannot use lambda in annotation scope within class scope"),
            "{error}"
        );
        let range = error.range().unwrap();
        assert_eq!(
            &source[usize::from(range.start())..usize::from(range.end())],
            "lambda: x"
        );
    }

    for alias in [
        "type Alias = ((value := x), lambda: x)",
        "type Alias = ((yield x), lambda: x)",
        "type Alias[T, T] = lambda: x",
    ] {
        let source = format!("class C:\n    {alias}\n");
        let error =
            compile_with_options(&source, "class_alias_nested_capture.py", &options).unwrap_err();
        assert!(!error.to_string().contains("Cannot use lambda"), "{error}");
    }

    for source in [
        "type Alias = lambda: x\n",
        "def outer(x):\n    type Alias = lambda: x\n    return Alias\n",
        "class C:\n    def method(self):\n        type Alias = lambda: x\n        return Alias\n",
    ] {
        compile_with_options(source, "class_alias_nested_capture.py", &options).unwrap();
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
