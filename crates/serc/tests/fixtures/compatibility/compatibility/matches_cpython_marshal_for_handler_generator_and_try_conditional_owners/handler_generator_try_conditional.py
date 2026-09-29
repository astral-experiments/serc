def optimized_any_in_handler(matches):
    try:
        result = matches[0]
    except IndexError:
        if any("*" in match for match in matches):
            expanded = []
            for match in matches:
                if "*" in match:
                    try:
                        expanded.append(int(match))
                    except ValueError:
                        expanded.append(match)
                else:
                    expanded.append(match)
            matches = expanded
        result = matches[0]
    return result


def conditional_try_marker(flag, values, index):
    try:
        result = build(values) if flag else values[index]
    except Error:
        recover()
    return result
