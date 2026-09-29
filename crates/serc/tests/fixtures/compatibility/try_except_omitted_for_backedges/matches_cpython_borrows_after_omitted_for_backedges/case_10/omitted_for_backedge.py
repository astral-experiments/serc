def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                with cm:
                    return item
            else:
                with cm:
                    return item
    use(value)
