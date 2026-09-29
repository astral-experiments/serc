def f():
    try:
        action()
    except X:
        first()
    except f(g(h())) or B:
        recover()
