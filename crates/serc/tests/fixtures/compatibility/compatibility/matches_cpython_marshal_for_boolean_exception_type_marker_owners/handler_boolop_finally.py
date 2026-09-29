def f():
    try:
        try:
            action()
        except A and B:
            recover()
    finally:
        cleanup()
