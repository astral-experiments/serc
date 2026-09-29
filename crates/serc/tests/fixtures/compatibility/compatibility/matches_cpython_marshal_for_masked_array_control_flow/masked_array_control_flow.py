def flatten(sequence):
    try:
        for element in sequence:
            if hasattr(element, "__iter__"):
                yield from flatten(element)
            else:
                yield element
    except TypeError:
        yield sequence


def view(dtype, type_, output, fill):
    if dtype is None:
        output = convert(dtype)
    elif type_ is None:
        try:
            if issubclass(dtype, object):
                output = convert(dtype)
                dtype = None
            else:
                output = convert(dtype)
        except TypeError:
            output = convert(dtype)
    else:
        output = convert(dtype)

    if output is not None:
        if fill is None:
            if dtype is None:
                pass
            else:
                output.fill = None
        else:
            output.fill = fill
    return output


def filled(action, self, mask, result):
    try:
        action()
    except (TypeError, AttributeError):
        result = fallback()
    except IndexError:
        if self.data.shape:
            raise
        elif mask:
            result = replacement()
        else:
            result = self.data
    return result
