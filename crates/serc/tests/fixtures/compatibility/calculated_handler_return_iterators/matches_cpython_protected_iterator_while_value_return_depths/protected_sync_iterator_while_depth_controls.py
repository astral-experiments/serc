def direct_iterator(values, done):
    try:
        for value in values:
            if done(value):
                return {"items": value}
    except Exception:
        recover()
    return None


def literal_under_while(values, done):
    try:
        for value in values:
            while ready(value):
                if done(value):
                    return None
                advance(value)
    except Exception:
        recover()
    return None


def newer_try_inside_iterator(values, done):
    try:
        for value in values:
            try:
                while ready(value):
                    if done(value):
                        return {"items": value}
                    advance(value)
            except ValueError:
                recover_inner()
    except Exception:
        recover()
    return None
