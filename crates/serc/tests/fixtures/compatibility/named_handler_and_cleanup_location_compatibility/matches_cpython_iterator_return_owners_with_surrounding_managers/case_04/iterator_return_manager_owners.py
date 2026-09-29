def f(items, other, manager):
    with manager:
        try:
            for item in items:
                for value in other:
                    try:
                        return work(item, value)
                    except Error:
                        recover(value)
        finally:
            finish()
