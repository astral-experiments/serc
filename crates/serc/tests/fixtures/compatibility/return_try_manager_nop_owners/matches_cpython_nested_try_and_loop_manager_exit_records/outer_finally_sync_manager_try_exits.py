def f(manager):
    try:
        while True:
            with manager:
                try:
                    if retry():
                        continue
                    return 70
                except ValueError:
                    recover()
            break
    finally:
        cleanup()
