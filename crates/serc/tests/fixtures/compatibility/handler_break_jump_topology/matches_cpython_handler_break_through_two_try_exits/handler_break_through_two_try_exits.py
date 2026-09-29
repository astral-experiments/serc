def f(values, enabled):
    try:
        if enabled[0]:
            try:
                if enabled[1]:
                    for value in values:
                        refresh(value)
                        try:
                            consume(value)
                        except LookupError:
                            break
            except RuntimeError:
                pass
    except RuntimeError:
        pass
    return None
