def f(items, attempts, ready):
    for attempt in attempts:
        for item in items:
            work(item)
            if ready():
                break
        else:
            prepare()
            if not ready():
                return attempt
            else:
                pass
    return None
