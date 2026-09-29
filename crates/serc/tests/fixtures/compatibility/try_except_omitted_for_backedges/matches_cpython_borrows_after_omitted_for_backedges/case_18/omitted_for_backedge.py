def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                pass
    use(value)
