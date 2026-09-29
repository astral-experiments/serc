def f(left, right):
    try:
        while True:
            a = left.poll()
            b = right.poll()
            if a is not None or b is not None:
                if a not in (None, 0):
                    code_a: int = a if a is not None else -1
                    raise Problem(code_a)
                if b not in (None, 0):
                    code_b: int = b if b is not None else -1
                    raise Problem(code_b)
                break
            tick()
    except Interrupt:
        pass
    finally:
        close(left)
        close(right)
