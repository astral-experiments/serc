def f(manager, running):
    try:
        with manager:
            while running():
                step()
    finally:
        finish()
