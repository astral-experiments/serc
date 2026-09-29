def f(manager):
    try:
        with manager:
            while True:
                step()
    finally:
        finish()
