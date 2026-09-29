def f(a,b,c,items):
    try:
        for item in items:
            if a:
                if b:
                    consume(item)
                elif c:
                    consume(item)
                else:
                    assert b
            else:
                consume(item)
        z = 1
    except ValueError:
        recover()
