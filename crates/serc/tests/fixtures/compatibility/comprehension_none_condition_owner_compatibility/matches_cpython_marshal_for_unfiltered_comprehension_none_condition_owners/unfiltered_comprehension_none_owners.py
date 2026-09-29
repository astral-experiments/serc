
def name_is_none(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacement):
    return [replacement if value is None else value for value in values]


def name_is_not_none(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, replacement):
    return [value if value is not None else replacement for value in values]


def call_and_subscript(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, mapping, iterator):
    return [next(iterator) if value is None else mapping[value] for value in values]


def nested_truthiness(a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, values, mapping, ready):
    return [(mapping[value] if value is not None else None) if ready else value for value in values]
