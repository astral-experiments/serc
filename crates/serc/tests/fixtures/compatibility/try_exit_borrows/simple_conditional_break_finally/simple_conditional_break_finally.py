def f(item):
    try:
        while True:
            value = item.poll()
            if value is not None:
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(item)
        finish(item)
