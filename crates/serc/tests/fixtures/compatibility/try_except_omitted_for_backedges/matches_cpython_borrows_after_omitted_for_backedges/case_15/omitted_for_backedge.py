def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                value = work(item)
            else:
                try:
                    return work(item)
                except Exception:
                    return item
    use(value)
