def probe(values, enabled, fail, collect):
    raise fail(
        code=1,
        values=(
            collect(values)
            if enabled
            else ()
        ),
    )
