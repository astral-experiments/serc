def f(a,b,c,items):
    try:
        for item in items:
            if a:
                if b:
                    consume(item)
                elif c:
                    assert c
                else:
                    assert b
            else:
                consume(item)
        else:
            finish()
        z = 1
    except ValueError:
        recover()
