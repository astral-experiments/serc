def f(manager, other):
    try:
        with manager as value: pass; pass; global marker
    finally:
        cleanup()
    return value
