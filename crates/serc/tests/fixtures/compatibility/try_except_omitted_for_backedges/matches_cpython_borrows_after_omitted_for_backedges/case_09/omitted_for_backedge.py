def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag is not None:
                value = work(item)
            else:
                value = item
    use(value)
