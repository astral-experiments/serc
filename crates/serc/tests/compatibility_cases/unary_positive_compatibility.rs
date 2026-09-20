use crate::common;

#[test]
fn matches_cpython_marshal_for_unary_positive_constants() {
    for expression in [
        "+True",
        "+False",
        "++True",
        "+0",
        "+42",
        "+(-42)",
        "+18446744073709551616",
        "+(-18446744073709551616)",
        "+1.5",
        "+(-0.0)",
        "+1j",
        "+'text'",
        "+b'bytes'",
        "+()",
        "+(1, 2)",
        "+None",
        "+...",
    ] {
        let source = format!("result = {expression}\n");
        common::assert_matches_cpython_marshal(&source, "unary_positive_constants.py");
    }
}

#[test]
fn unary_positive_preserves_types_and_errors() {
    let source = r"print(type(+True).__name__, +True, type(+False).__name__, +False)
for operation in (
    lambda: +'text',
    lambda: +b'bytes',
    lambda: +(1, 2),
    lambda: +None,
    lambda: +...,
):
    try:
        operation()
    except TypeError:
        print('TypeError')
    else:
        print('missing TypeError')
";
    common::assert_executes(
        source,
        "int 1 int 0\nTypeError\nTypeError\nTypeError\nTypeError\nTypeError\n",
    );
}
