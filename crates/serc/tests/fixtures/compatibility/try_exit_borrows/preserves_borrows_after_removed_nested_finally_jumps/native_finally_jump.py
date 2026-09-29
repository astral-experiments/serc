def f(flag, guard, value, cm):
    try:
        action(value)
    finally:
        if guard:
            if guard:
                try:
                    if flag:
                        action(value)
                except Exception:
                    pass
    consume(value)
