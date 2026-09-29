def collect(values):
    output = []
    for value in values:
        match value:
            case Included():
                output.append(value)
            case Skipped():
                pass
            case _:
                assert_never(value)
    return output
