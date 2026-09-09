use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_typed_handler_entry_owner_boundaries() {
    for (declaration, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for handler in [
            "except:",
            "except Error:",
            "except (Error, OtherError):",
            "except Error as error:",
        ] {
            for prefix in ["consume(flag)", "work()"] {
                let source = format!(
                    r"{declaration} f(flag, consume):
{suspension}    try:
        work()
    {handler}
        {prefix}
        try:
            result = 1 if flag else 2
        except Error:
            pass
"
                );
                assert_matches_cpython_marshal(&source, "typed_handler_entry_owner_boundary.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_unnamed_handler_continuations_into_retained_setups() {
    for (declaration, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for handler in ["except Error:", "except (Error, OtherError):"] {
            for body in [
                "        try:\n            result = 1 if flag else 2\n        except Error:\n            pass\n",
                "        try:\n            if flag:\n                consume(flag)\n        finally:\n            consume(flag)\n        raise\n",
                "        work()\n        try:\n            if flag:\n                work()\n        except Error:\n            pass\n",
            ] {
                let source = format!(
                    "{declaration} f(flag, consume):\n{suspension}    try:\n        work()\n    {handler}\n{body}"
                );
                assert_matches_cpython_marshal(&source, "unnamed_handler_continuation.py");
            }
        }
    }
}
