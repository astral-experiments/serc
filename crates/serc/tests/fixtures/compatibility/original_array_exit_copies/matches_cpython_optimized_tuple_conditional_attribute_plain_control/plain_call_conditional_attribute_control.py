def probe(obj, values, collect):
    obj.value = (
        collect(values)
        if values
        else (0.0,)
    )
