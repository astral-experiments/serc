use crate::common;

#[test]
fn matches_cpython_annotation_lambda_captures() {
    for expression in [
        "lambda: value",
        "lambda: lambda: value",
        "lambda arg=value: arg",
        "lambda value: value",
        "lambda value: lambda: value",
    ] {
        for declaration in [
            format!("def f(x: {expression}):\n        pass\n    return f\n"),
            format!("def f() -> {expression}:\n        pass\n    return f\n"),
            format!(
                "def f(pos: {expression}, /, *args: {expression}, named: {expression}, **kwargs: {expression}):\n        pass\n    return f\n"
            ),
            format!("class C:\n        x: {expression}\n    return C\n"),
            format!("class C:\n        def f(x: {expression}):\n            pass\n    return C\n"),
        ] {
            let source = format!("def outer(value):\n    {declaration}");
            common::assert_matches_cpython_marshal(&source, "annotation_lambda_captures.py");
            common::assert_matches_cpython_marshal(
                &format!("from __future__ import annotations\n{source}"),
                "annotation_lambda_strings.py",
            );
        }
    }
}

#[test]
fn executes_annotation_lambda_captures() {
    let source = r"def outer(value):
    def f(x: lambda: value):
        pass
    class C:
        x: lambda: value
        def method(x: lambda: value):
            pass
    return f, C

f, C = outer(42)
print(f.__annotations__['x']())
print(C.__annotations__['x']())
print(C.method.__annotations__['x']())
";
    common::assert_executes(source, "42\n42\n42\n");
}

#[test]
fn annotation_lambdas_capture_enclosing_names_despite_class_globals() {
    for source in [
        "def outer(x):\n class C:\n  x = 10\n  def f(a: lambda: x): pass\n return C\n",
        "def outer(x):\n class C:\n  global x\n  def f(a: lambda: x): pass\n return C\n",
        "def outer(x):\n class C:\n  global x\n  a: lambda: x\n return C\n",
        "def outer(x):\n class C:\n  global x\n  def f(a: (x, lambda: x)): pass\n return C\n",
        "def outer(x):\n class C:\n  global x\n  a: (x, lambda: x)\n return C\n",
        "class C:\n global __class__\n def f(a: lambda: __class__): pass\n",
        "class C:\n global __class__\n a: lambda: __class__\n",
        "class C:\n global T\n def f[T](a: lambda: T): pass\n",
        "class C:\n global T\n def f[T](a: (T, lambda: T)): pass\n",
    ] {
        common::assert_matches_cpython_marshal(source, "annotation_lambda_globals.py");
    }
    common::assert_executes(
        "x = 'global'\ndef outer(x):\n class C:\n  global x\n  a: (x, lambda: x)\n  def f(a: (x, lambda: x)): pass\n return C\nC = outer('outer')\nfor annotations in (C.__annotations__, C.f.__annotations__):\n direct, nested = annotations['a']\n print(direct, nested())\n",
        "global outer\nglobal outer\n",
    );
}

#[test]
fn function_annotations_keep_multiple_shadowed_nested_captures() {
    let source = r"def outer(value, other):
    def plain(a: lambda: value, b: [other for _ in ()]):
        pass
    class Bound:
        value = int
        other = str
        def method(a: (value, lambda: value), b: (other, lambda: other)):
            pass
    class Global:
        global value, other
        def method(a: (value, lambda: value), b: (other, lambda: other)):
            pass
    return plain, Bound, Global
";
    for future in ["", "from __future__ import annotations\n"] {
        common::assert_matches_cpython_marshal(
            &format!("{future}{source}"),
            "function_annotation_shadowed_captures.py",
        );
    }
}

#[test]
fn discarded_local_annotations_preserve_only_nested_captures() {
    for expression in [
        "value",
        "lambda: value",
        "lambda arg=value: arg",
        "lambda arg=(lambda: value): arg",
        "[value for item in ()]",
        "{value for item in ()}",
        "{item: value for item in ()}",
        "(value for item in ())",
        "[item for item in value]",
        "(item for item in value)",
        "[value for value in ()]",
        "[(lambda: item) for item in ()]",
        "lambda: later",
        "[later for item in ()]",
    ] {
        for target in ["ignored", "obj.attribute", "obj[0]"] {
            for nested in [false, true] {
                let declaration = format!("{target}: {expression}\n");
                let source = if nested {
                    format!(
                        "def outer(value, obj):\n    def inner():\n        {declaration}    later = 1\n    return inner\n"
                    )
                } else {
                    format!("def outer(value, obj):\n    {declaration}    later = 1\n")
                };
                for future in ["", "from __future__ import annotations\n"] {
                    common::assert_matches_cpython_marshal(
                        &format!("{future}{source}"),
                        "discarded_annotation_captures.py",
                    );
                }
            }
        }
    }
}

#[test]
fn discarded_local_annotations_expose_cells_without_evaluating_the_annotation() {
    let source = r"class C:
    def method(self, __value):
        ignored: [(__value, self) for item in ()]
        return tuple(sorted(locals()))

def outer(value):
    def inner():
        ignored: (lambda: value) if False else (value for item in ())
    return inner

print(C.method.__code__.co_cellvars)
print(C().method(42))
print(outer.__code__.co_cellvars)
print(outer(42).__code__.co_freevars)
print(outer(42)())
";
    common::assert_matches_cpython_marshal(source, "discarded_annotation_runtime.py");
    common::assert_executes(
        source,
        "('self', '_C__value')\n('_C__value', 'self')\n('value',)\n('value',)\nNone\n",
    );
}
