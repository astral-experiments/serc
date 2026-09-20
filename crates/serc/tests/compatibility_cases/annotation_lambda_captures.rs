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
