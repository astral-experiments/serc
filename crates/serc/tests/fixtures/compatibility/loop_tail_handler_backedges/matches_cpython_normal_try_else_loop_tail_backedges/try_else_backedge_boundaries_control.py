def with_finally(items, output):
    for item in items:
        try:
            value = item.read()
        except ValueError:
            pass
        else:
            if value:
                output.append(value)
        finally:
            item.close()
    return output


def folded_tail(items, output):
    for item in items:
        try:
            value = item.read()
        except ValueError:
            pass
        else:
            if False:
                output.append(value)
    return output
