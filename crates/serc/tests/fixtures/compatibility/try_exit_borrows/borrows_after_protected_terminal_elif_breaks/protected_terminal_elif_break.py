def f(item):
    try:
        while True:
            state = read()
            if state < 0:
                use(item)
            elif state == 0:
                break
            elif state == 1:
                pause()
                break
        use(item)
        use(item)
        return item
    except Error as exc:
        report(exc)
        return None
    finally:
        cleanup(item)
