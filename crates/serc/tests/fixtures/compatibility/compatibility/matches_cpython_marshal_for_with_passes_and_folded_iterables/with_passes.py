def pass_before_loop(manager, values):
    with manager as target:
        pass
        for value in values:
            target.write(value)

def pass_in_loop_and_else(manager, values):
    with manager as target:
        for value in values:
            pass
            target.write(value)
        else:
            pass

def folded_iterable(manager):
    with manager as target:
        for value in (1,) if True else (2,):
            target.write(value)

with (
    manager
) as target: pass
