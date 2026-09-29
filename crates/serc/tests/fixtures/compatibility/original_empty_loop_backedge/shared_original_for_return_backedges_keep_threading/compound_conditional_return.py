def f(items, attempts, first, second):
    for attempt in attempts:
        for item in items:
            work(item)
            if first():
                break
        else:
            prepare()
            if first() and second():
                return attempt
    return None
