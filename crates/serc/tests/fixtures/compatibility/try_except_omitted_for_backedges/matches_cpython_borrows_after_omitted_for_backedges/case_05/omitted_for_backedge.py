def probe(x, flag, work, use):
    try:
        value = None
        for item in x:
            if flag:
                value = item
                break
    except Exception:
        value = None
        if value is None:
            raise ValueError()
    use(value)
