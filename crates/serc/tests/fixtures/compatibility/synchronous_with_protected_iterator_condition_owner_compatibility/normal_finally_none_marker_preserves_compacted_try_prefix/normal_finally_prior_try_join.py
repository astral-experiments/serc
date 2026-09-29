def f(manager, flag, metric, ready):
    def capture():
        return flag
    with manager:
        try:
            if ready:
                begin()
            else:
                prepare()
            a = int(metric.value)
            b = int(metric.value)
            c = int(metric.value)
            d = int(metric.value)
            e = int(metric.value)
            f = int(metric.value)
            g = int(metric.value)
        finally:
            if flag is not None:
                flag.close()
    return a, b, c, d, e, f, g, capture
