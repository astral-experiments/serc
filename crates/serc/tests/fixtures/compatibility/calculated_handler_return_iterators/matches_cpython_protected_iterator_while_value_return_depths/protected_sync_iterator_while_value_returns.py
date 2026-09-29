def one_iterator(values, done):
    try:
        for value in values:
            while ready(value):
                if done(value):
                    return {"items": value}
                advance(value)
    except Exception:
        recover()
    return None


def two_iterators(rows, done):
    try:
        for row in rows:
            while ready(row):
                for value in row:
                    while ready(value):
                        if done(value):
                            return {"items": value}
                        advance(value)
                advance(row)
    except Exception:
        recover()
    return None
