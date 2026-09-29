convert = lambda value: factory()(
    transform(value) if fast(value) and valid(value) else value
)
