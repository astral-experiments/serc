def f():
    yield source()
    try:
        action()
    except A and B:
        recover()
