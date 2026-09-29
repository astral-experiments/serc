def f(items, manager):
    with manager:
        try:
            work()
        finally:
            for item in items:
                if item.left == LIMIT and item.right == "ready":
                    consume(item)
