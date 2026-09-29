convert = lambda value: (
    value
    if is_ready(value)
    else factory()(
        transform(value) if fast(value) and valid(value) else value
    )
)
