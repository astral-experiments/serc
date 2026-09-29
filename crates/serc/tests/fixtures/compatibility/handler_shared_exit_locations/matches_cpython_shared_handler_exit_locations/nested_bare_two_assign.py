def f(values, flag):
    for value in values:
        try:
            try:
                work()
            except Exception:
                raw = read()
                if raw:
                    result = transform(raw)
                    value = raw
            if flag:
                finish(value)
        except Exception:
            recover()
