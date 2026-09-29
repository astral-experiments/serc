def multiline_and(manager, value, expected):
    with manager:
        return (
            value is not None
            and value.kind == expected.kind
            and value.name == expected.name
            and value.flags == expected.flags
        )
