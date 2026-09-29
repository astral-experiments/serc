def f(x):
    try:
        consume(
            x,
        ); pass; pass
    except ValueError:
        recover()
