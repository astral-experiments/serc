def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
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
