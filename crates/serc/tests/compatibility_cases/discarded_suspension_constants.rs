use crate::common;

#[test]
fn matches_cpython_marshal_for_discarded_suspension_constants() {
    for suspension in [
        "yield",
        "yield from values",
        "yield from produce('operand', key='keyword')",
        "yield 'operand'",
        "yield None",
    ] {
        for prefix in ["", "    'docstring'\n", "    consume(None)\n"] {
            for discarded in [
                format!("    if flag:\n        raise Error\n        {suspension}\n"),
                format!("    if flag:\n        raise Error('early')\n        {suspension}\n"),
                format!("    if False:\n        {suspension}\n"),
                format!("    while False:\n        {suspension}\n"),
            ] {
                let source = format!(
                    "def f(flag):\n{prefix}{discarded}    consume('later')\n    yield from values\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "discarded_suspension_constants.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_nested_discarded_suspension_operands() {
    for source in [
        "def f(flag):\n    if flag:\n        raise Error\n        consume((yield) or 'after operand')\n    consume('later')\n    yield values\n",
        "def f(flag):\n    if flag:\n        raise Error\n        yield from ((yield 'inner') or 'after operand')\n    consume('later')\n    yield values\n",
        "def f(flag):\n    if flag:\n        raise Error('early')\n        yield from (lambda: 'child')()\n    consume('later')\n    yield values\n",
        "def f(flag):\n    if flag:\n        raise Error('early')\n        consume(item for item in values)\n    consume('later')\n    yield values\n",
        "async def f(flag):\n    if flag:\n        raise Error\n        yield\n    consume('later')\n    yield values\n",
        "async def f(flag):\n    if flag:\n        raise Error\n        await produce('operand')\n    consume('later')\n    yield values\n",
        "async def f(flag):\n    if flag:\n        raise Error\n        yield 'operand'\n    consume('later')\n    yield values\n",
    ] {
        common::assert_matches_cpython_marshal(source, "nested_discarded_suspension_operands.py");
    }
}
