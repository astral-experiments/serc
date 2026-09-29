def f(factory, other):
    try:
        return factory()
    finally:
        return other()
