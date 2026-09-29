def multiple_fallthrough_edges(manager, source, expected):
    try:
        with manager:
            values = source()
            if len(values) == 2 and int(values[1]) == expected:
                return True
    except Error:
        pass
    after()


def single_fallthrough_edge(manager, first, second):
    with manager:
        value = make()
        if not first or not second:
            raise Error(value)
