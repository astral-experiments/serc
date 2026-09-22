def after():
    x = 42
    return (([x for x in (1, 2)], x) for _ in [0])

def empty():
    x = 42
    return ((x, [x for x in ()]) for _ in [0])

def filter():
    x = 42
    return (x for _ in [0] if [x for x in (1,)])

def first_iterable():
    x = (1, 2)
    return ([x for x in x] for _ in [0])

def inlined_cell():
    x = 42
    return ((x, [lambda: x for x in (1,)]) for _ in [0])

def later_iterable():
    x = 42
    return (x for _ in [0] for tail in [x for x in (1,)])

def local_control():
    x = 42
    return ((x, [x for x in ()]) for x in [42])

def nonempty():
    x = 42
    return ((x, [x for x in (1, 2)]) for _ in [0])

def outer_iterable_control():
    x = 42
    return (x for x in [x for x in (1, 2)])

def renamed_control():
    x = 42
    return ((x, [y for y in ()]) for _ in [0])
