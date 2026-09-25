use serc::{CompileOptions, PythonVersion, compile_with_options};

#[test]
fn python_312_rejects_nested_scopes_in_class_visible_generic_headers() {
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for (expression, kind) in [
        ("lambda: 1", "lambda"),
        ("[x for x in ()]", "comprehension"),
        ("{x for x in ()}", "comprehension"),
        ("{x: x for x in ()}", "comprehension"),
        ("(x for x in ())", "comprehension"),
    ] {
        for header in [
            format!("class C[T: ({expression})]: pass"),
            format!("class C[T](({expression})): pass"),
            format!("class C[T](metaclass=({expression})): pass"),
            format!("def f[T: ({expression})](): pass"),
            format!("async def f[T: ({expression})](): pass"),
            format!("def f[T](x: ({expression})): pass"),
            format!("def f[T]() -> ({expression}): pass"),
            format!("type Alias[T: ({expression})] = int"),
        ] {
            let source = format!("class Outer:\n    {header}\n");
            let error =
                compile_with_options(&source, "class_annotation.py", &options).expect_err(&source);
            assert!(
                error.to_string().contains(&format!(
                    "Cannot use {kind} in annotation scope within class scope"
                )),
                "{source}\n{error}"
            );
            let range = error.range().unwrap();
            assert_eq!(
                &source[usize::from(range.start())..usize::from(range.end())],
                expression,
                "{source}"
            );

            for python_version in [
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                compile_with_options(
                    &source,
                    "class_annotation.py",
                    &CompileOptions {
                        python_version,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_or_else(|error| panic!("{source}\n{python_version:?}: {error}"));
            }
        }
    }
}

#[test]
fn class_annotation_restrictions_follow_symbol_analysis_order() {
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for (header, expected) in [
        (
            "@(lambda x, x: 1)\n    class C[T: (lambda: 1)]: pass",
            "Duplicate parameter",
        ),
        (
            "def f[T: (lambda: 1)](x=(lambda y, y: 1)): pass",
            "Duplicate parameter",
        ),
        (
            "def f[T: (lambda: 1)](x=(await g())): pass",
            "Cannot use lambda",
        ),
        (
            "@(await g())\n    class C[T: (lambda: 1)]: pass",
            "Cannot use lambda",
        ),
        ("class C[T: (lambda: 1), T]: pass", "Cannot use lambda"),
        (
            "class C[T: ([x for x in ()], lambda: 1)]: pass",
            "Cannot use comprehension",
        ),
        (
            "class C[T: (lambda: 1, (value := 1))]: pass",
            "Cannot use lambda",
        ),
        (
            "class C[T, T: (lambda: 1)]: pass",
            "duplicate type parameter",
        ),
        ("class C[__debug__: (lambda: 1)]: pass", "Cannot use lambda"),
        ("class __debug__[T: (lambda: 1)]: pass", "Cannot use lambda"),
        ("type __debug__ = lambda: 1", "Cannot use lambda"),
        (
            "class C[T: {0: (lambda: 1), (value := 1): 2}]: pass",
            "named expression",
        ),
        (
            "def f[T](*, x: (lambda: 1), **kw: [x for x in ()]): pass",
            "Cannot use comprehension",
        ),
    ] {
        let source = format!("class Outer:\n    {header}\n");
        let error = compile_with_options(&source, "annotation_order.py", &options).unwrap_err();
        assert!(error.to_string().contains(expected), "{source}\n{error}");
    }
}

#[test]
fn class_annotation_restrictions_do_not_escape_their_scope() {
    let options = CompileOptions {
        python_version: PythonVersion::Py312,
        ..CompileOptions::default()
    };
    for source in [
        "class C[T: (lambda: 1)]: pass\n",
        "def f[T: [x for x in ()]](): pass\n",
        "class Outer:\n    class C((lambda: int)()): pass\n",
        "class Outer:\n    def f(x: (lambda: 1)) -> [x for x in ()]: pass\n",
        "class Outer:\n    def f[T](x=(lambda: 1)): pass\n",
        "class Outer:\n    @decorate(lambda: 1)\n    class C[T]: callback = lambda: T\n",
        "class Outer:\n    def method(self):\n        class C[T: (lambda: 1)]: pass\n",
        "class Outer:\n    def method(self):\n        def f[T: [x for x in ()]](): pass\n",
        "from __future__ import annotations\nclass Outer:\n    def f[T](x: (lambda: 1)) -> [x for x in ()]: pass\n",
    ] {
        compile_with_options(source, "annotation_scope_control.py", &options)
            .unwrap_or_else(|error| panic!("{source}\n{error}"));
    }
    let source =
        "from __future__ import annotations\nclass Outer:\n    def f[T: (lambda: 1)](): pass\n";
    let error = compile_with_options(source, "annotation_bound.py", &options).unwrap_err();
    assert!(error.to_string().contains("Cannot use lambda"), "{error}");
}
