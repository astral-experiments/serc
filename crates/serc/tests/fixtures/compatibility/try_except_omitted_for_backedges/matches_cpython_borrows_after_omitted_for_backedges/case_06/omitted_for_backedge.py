def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            pass
        if value is None:
            raise ValueError()
    use(value)
