def f(x):
    try:
        consume(
            x,
        ); pass
    except ValueError:
        recover()
