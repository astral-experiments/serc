def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
            if stop:
                break
    return False
