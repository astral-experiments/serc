
def list_filter(outer, values, flag):
    return [tuple(x for x in values) for y in outer if flag]
