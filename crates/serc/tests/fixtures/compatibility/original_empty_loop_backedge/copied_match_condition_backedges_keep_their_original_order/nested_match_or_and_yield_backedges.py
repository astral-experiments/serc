def f(groups, left, right):
    for group in groups:
        for value in group:
            match value:
                case [*items] | (*items,):
                    if any(left(item) for item in items) and not any(
                        right(item) for item in items
                    ):
                        yield value
