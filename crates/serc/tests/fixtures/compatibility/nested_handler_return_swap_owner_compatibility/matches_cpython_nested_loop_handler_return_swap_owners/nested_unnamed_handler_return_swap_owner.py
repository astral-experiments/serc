def example(groups):
    for group in groups:
        for value in group:
            try:
                return primary(value)
            except Exception:
                try:
                    return secondary(value)
                except Exception:
                    try:
                        return tertiary(value)
                    except Exception:
                        continue
