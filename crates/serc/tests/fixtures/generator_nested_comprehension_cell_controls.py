def cell_control():
    x = 42
    return (((lambda: x), [x for x in ()]) for x in [42])

def outside_lambda():
    x = 42
    return (((lambda: x), [x for x in ()]) for _ in [0])
