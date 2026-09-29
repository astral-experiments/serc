def f(item):
    try:
        while True:
            value = item.poll()
            if value is not None:
                if value:
                    raise Problem(value)
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(item)
        finish(item)
