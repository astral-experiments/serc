def or_else_group(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b or c
        )) or d
    )


def or_body_group(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a or b
        ) if condition else c) or d
    )


def or_both_groups(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a or b
        ) if condition else (
            c or d
        )) or e
    )


def or_nested_else(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b if other_condition else (
                c or d
            )
        )) or e
    )


def or_nested_body(a, b, c, d, e, condition, other_condition):
    return (
        ((
            (a or b) if other_condition else c
        ) if condition else d) or e
    )


def or_trailing_group(a, b, c, d, e, condition, other_condition):
    return (
        (a and (
            b if condition else (
                c or d
            )
        )) or e
    )


def or_mixed_operator(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b and c
        )) or d
    )


def or_negated_value(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else not (
            b or c
        )) or d
    )


def or_negated_condition(a, b, c, d, e, condition, other_condition):
    return (
        (a if not (
            condition and other_condition
        ) else (
            b or c
        )) or d
    )


def or_folded_true(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a or b
        ) if True else c) or d
    )


def or_folded_false(a, b, c, d, e, condition, other_condition):
    return (
        (a if False else (
            b or c
        )) or d
    )


def and_else_group(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b and c
        )) and d
    )


def and_body_group(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a and b
        ) if condition else c) and d
    )


def and_both_groups(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a and b
        ) if condition else (
            c and d
        )) and e
    )


def and_nested_else(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b if other_condition else (
                c and d
            )
        )) and e
    )


def and_nested_body(a, b, c, d, e, condition, other_condition):
    return (
        ((
            (a and b) if other_condition else c
        ) if condition else d) and e
    )


def and_trailing_group(a, b, c, d, e, condition, other_condition):
    return (
        (a or (
            b if condition else (
                c and d
            )
        )) and e
    )


def and_mixed_operator(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else (
            b or c
        )) and d
    )


def and_negated_value(a, b, c, d, e, condition, other_condition):
    return (
        (a if condition else not (
            b and c
        )) and d
    )


def and_negated_condition(a, b, c, d, e, condition, other_condition):
    return (
        (a if not (
            condition or other_condition
        ) else (
            b and c
        )) and d
    )


def and_folded_true(a, b, c, d, e, condition, other_condition):
    return (
        ((
            a and b
        ) if True else c) and d
    )


def and_folded_false(a, b, c, d, e, condition, other_condition):
    return (
        (a if False else (
            b and c
        )) and d
    )


def protected_result(a, b, c, d, condition):
    try:
        value = (a if condition else (
            b or c
        )) or d
        return value
    except ValueError:
        return d


async def coroutine_result(a, b, c, d, condition):
    return consume((a if condition else (
        b or c
    )) or d)


def generator_result(a, b, c, d, condition):
    yield (a if condition else (
        b and c
    )) and d
