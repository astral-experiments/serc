from contextlib import suppress

def f():
    with suppress(Exception):
        if 1:
            pass
        elif 1:
            pass
        elif 1:
            pass
        try:
            pass
        except Exception:
            pass
        finally:
            pass
        if 2:
            pass
        while True:
            pass
