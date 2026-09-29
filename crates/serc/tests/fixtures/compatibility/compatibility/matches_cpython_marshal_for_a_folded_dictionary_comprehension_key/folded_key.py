def folded_key():
    return {x if True else y: y for x in range(10) for y in range(10)}
