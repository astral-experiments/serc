def f(items, manager, other_manager, value):
    try:
        for item in items:
            with manager, other_manager:
                return item, value
    finally:
        finish()
