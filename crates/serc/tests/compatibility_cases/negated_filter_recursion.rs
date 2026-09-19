use crate::common;

#[test]
fn matches_cpython_recursive_negated_comprehension_filters() {
    for expression in [
        "[x for x in items if CONDITION]",
        "{x for x in items if CONDITION}",
        "{x: transform(x) for x in items if CONDITION}",
        "(x for x in items if CONDITION)",
    ] {
        for condition in [
            "not not (first(x) or second(x))",
            "not not (first(x) and second(x))",
            "not not not (first(x) or second(x))",
            "not (first(x) or not second(x))",
            "not ((first(x) and second(x)) or third(x))",
            "not (first(x) or (second(x) and third(x)))",
            "not (first(x) and not (second(x) or third(x)))",
            "not (first(x) and second(x) and not third(x))",
            "not (first(x) or (second(x) and not third(x)))",
        ] {
            let expression = expression.replace("CONDITION", condition);
            let source = format!(
                "def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):\n    return {expression}\n"
            );
            common::assert_matches_cpython_marshal(&source, "negated_filter_recursion.py");
        }
    }
}

#[test]
fn matches_cpython_recursive_negated_filters_in_multiline_contexts() {
    for source in [
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    with manager:
        return [
            x
            for x in items
            if (
                not (first(x) or second(x))
            )
        ]
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        return {
            x
            for x in items
            if (
                not (first(x) or (second(x) and not third(x)))
            )
        }
    except Exception:
        return fallback()
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        return {
            x: transform(x)
            for x in items
            if (
                not (first(x) or second(x))
            )
        }
    finally:
        cleanup()
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        observe()
    except Exception as error:
        return [
            x
            for x in items
            if (
                not (first(x) and not (second(x) or third(x)))
            )
        ]
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    with manager:
        return (
            x
            for x in items
            if (
                not (first(x) or not second(x))
            )
        )
",
        r"async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return [
        x
        for x in items
        if (
            not (first(x) or (second(x) and third(x)))
        )
    ]
",
        r"async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return (
        x
        async for x in items
        if (
            not not x
        )
    )
",
        r"class Sample:
    result = {
        x: transform(x)
        for x in items
        if (
            not (first(x) or (second(x) and third(x)))
        )
    }
",
        r"result = {
    x
    for x in items
    if (
        not not (first(x) and second(x))
    )
}
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    return [
        x
        for x in items
        for y in others
        if keep(y)
        if (
            not not (first(x) or second(x))
        )
    ]
",
        r"async def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    async with manager:
        return [
            x
            for x in items
            if (
                not not (first(x) or second(x))
            )
        ]
",
        r"def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
    try:
        observe()
    except* Exception as error:
        result = (
            x
            for x in items
            if (
                not (first(x) or not second(x))
            )
        )
    return result
",
    ] {
        common::assert_matches_cpython_marshal(source, "multiline_negated_filter_recursion.py");
    }
}
