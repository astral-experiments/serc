def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        while x:
            if flag:
                value = x
                break
        if value is None:
            raise ValueError()
    use(value)
