def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                with cm:
                    return item
    use(value)
