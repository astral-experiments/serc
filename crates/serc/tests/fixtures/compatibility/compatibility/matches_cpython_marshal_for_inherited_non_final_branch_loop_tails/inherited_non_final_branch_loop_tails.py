def append_library(libraries, item):
    for existing in libraries:
        if is_sequence(existing):
            if is_sequence(item):
                if item[0] == existing[0]:
                    if item[1] is existing[1]:
                        return
                    warn(item[0])
                    break
            else:
                if item == existing[0]:
                    warn(item[0])
                    break
        else:
            if is_sequence(item):
                if item[0] == existing:
                    warn(item[0])
                    break
            else:
                if item == existing:
                    return
    libraries.append(item)


def final_else_control(items):
    for item in items:
        if is_sequence(item):
            consume(item)
        else:
            if accepted(item):
                consume(item)
