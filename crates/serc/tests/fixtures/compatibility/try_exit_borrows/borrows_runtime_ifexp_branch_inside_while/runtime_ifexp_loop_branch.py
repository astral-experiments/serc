def f(selector, first, second):
    while running():
        second = update()
        if (left(first, second) if selector else right(first, second)):
            return make(status='ok', first=first, second=second, flag=selector)
        after()
