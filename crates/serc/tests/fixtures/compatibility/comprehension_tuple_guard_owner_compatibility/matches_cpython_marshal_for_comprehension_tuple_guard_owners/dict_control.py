
def dict_control(outer, values):
    return {y: tuple(x for x in values) for y in outer}
