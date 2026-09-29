def probe(values, enabled, fail):
    raise fail(
        code=1,
        values=(
            tuple(value.code for value in values)
            if enabled
            else ()
        ),
    )
