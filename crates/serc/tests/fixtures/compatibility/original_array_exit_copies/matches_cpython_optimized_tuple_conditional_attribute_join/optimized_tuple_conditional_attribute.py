def probe(obj, values):
    obj.value = (
        tuple(float(value) for value in values)
        if values
        else (0.0,)
    )
