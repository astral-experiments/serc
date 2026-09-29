def f(flag, value):
    if flag:
        try:
            return primary(value)
        except ValueError:
            try:
                parsed = parse(value)
            except TypeError:
                return None
            if isinstance(parsed, str):
                prepared = parsed.strip()
                try:
                    return secondary(prepared)
                except TypeError:
                    return None
    return None
