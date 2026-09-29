def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
    return consume(items)
