def f(flag, values):
    try:
        if flag:
            for value in values:
                work(value)
        else:
            value = fallback()
    except Error:
        recover()
    finally:
        finish()
    completed()
