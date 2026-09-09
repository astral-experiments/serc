use crate::common;

#[test]
fn matches_cpython_first_discarded_child_constant() {
    for expression in [
        "consume(item for item in values)",
        "consume('child' for item in values)",
        "consume(item for item in ('iterable',))",
        "consume(item for item in (inner for inner in values))",
        "consume((item for item in values), 'trailing')",
        "consume(lambda: 'child')",
        "consume(lambda value='default': value)",
        "consume(lambda *, value='keyworddefault': value)",
        "consume([lambda: item for item in values])",
    ] {
        for (declaration, suspension) in [
            ("def", "return values"),
            ("def", "yield values"),
            ("async def", "await work()"),
            ("async def", "yield values"),
        ] {
            let source = format!(
                "{declaration} f(flag, values):\n    if flag:\n        raise Error\n        {expression}\n    consume('later')\n    {suspension}\n"
            );
            common::assert_matches_cpython_marshal(&source, "discarded_child_constant.py");
        }
    }
}

#[test]
fn matches_cpython_discarded_child_constant_boundaries() {
    for prefix in [
        "",
        "    'docstring'\n",
        "    consume('earlier')\n",
        "    consume(1)\n",
    ] {
        for dead in [
            "    if flag:\n        raise Error\n        consume(item for item in values)\n",
            "    while flag:\n        break\n        consume(item for item in values)\n",
            "    while flag:\n        continue\n        consume(item for item in values)\n",
            "    if False:\n        consume(item for item in values)\n",
            "    while False:\n        consume(item for item in values)\n",
        ] {
            let source = format!(
                "def f(flag, values):\n{prefix}{dead}    consume('later')\n    yield values\n"
            );
            common::assert_matches_cpython_marshal(&source, "discarded_child_boundaries.py");
        }
    }
    for source in [
        "def f(flag, values):\n    if flag:\n        raise Error\n        yield from (item for item in values)\n    consume('later')\n    yield values\n",
        "class Class:\n    def f(flag, values):\n        if flag:\n            raise Error\n            consume((lambda *, __value='default': __value), key='__value')\n        consume('later')\n        yield values\n",
        "def outer(captured):\n    def f(flag, values):\n        if flag:\n            raise Error\n            consume((lambda: captured) for item in values)\n        consume('later')\n        yield captured\n    return f\n",
    ] {
        common::assert_matches_cpython_marshal(source, "discarded_child_scope.py");
    }
}
