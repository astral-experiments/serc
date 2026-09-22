def exceptions(value):
    try:
        result = transform(value)
    except ValueError as error:
        result = recover(error)
    except (TypeError, RuntimeError):
        raise
    else:
        result += 1
    finally:
        cleanup()
    return result


def exception_groups(value):
    try:
        grouped(value)
    except* ValueError as errors:
        handle_values(errors)
    except* TypeError:
        handle_types()
