def probe(x, flag, work, use):
    try:
        value = work(x)
    except Exception:
        value = None
        for item in x:
            if flag:
                pass
            else:
                pass
    use(value)
