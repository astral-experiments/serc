
def list_call(outer, values):
    return [(seed(), tuple(x for x in values))[1] for y in outer]
