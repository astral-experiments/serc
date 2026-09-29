
def final_keyword(values, enabled):
    return Result(
        items=(
            tuple(value for value in values)
            if enabled
            else ()
        ),
    )


def penultimate_keyword(values, enabled, count):
    return Result(
        items=(
            tuple(value for value in values if value is not None)
            if enabled
            else ()
        ),
        count=count,
    )
