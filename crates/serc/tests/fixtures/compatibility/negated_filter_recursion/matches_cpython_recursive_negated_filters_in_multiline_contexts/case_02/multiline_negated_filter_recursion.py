def collect(items, others, first, second, third, fourth, flag, limit, manager, keep, transform):
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
