def probe(x, flag, work, use, cm):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                try:
                    return work(item)
                except Exception:
                    return item
            else:
                try:
                    return work(item)
                except Exception:
                    return item
    use(value)
