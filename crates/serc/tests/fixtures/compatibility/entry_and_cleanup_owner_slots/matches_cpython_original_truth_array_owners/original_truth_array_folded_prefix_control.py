def f(manager, values):
    total = 0
    with manager:
        for value in values:
            try:
                if value is not None:
                    choices = value.get("choices")
                    padding = (1, 2, 3)
                    if choices:
                        total += len(choices)
                    else:
                        total += len(padding)
            except Exception:
                continue
    return total
