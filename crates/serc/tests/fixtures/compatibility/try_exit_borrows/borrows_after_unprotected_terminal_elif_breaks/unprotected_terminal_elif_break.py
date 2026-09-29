def f(item):
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
