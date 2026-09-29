def f(obj, count):
    try:
        while condition() and count < 3:
            action()
        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
