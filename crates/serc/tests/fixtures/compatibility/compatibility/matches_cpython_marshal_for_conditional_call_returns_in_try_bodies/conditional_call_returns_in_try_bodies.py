def direct(flag):
    try:
        return build(value="a" if flag else "b")
    except Error:
        return failure()


def named_handler(flag):
    try:
        return build(value="a" if flag else "b")
    except Error as error:
        return failure(error)


def preceding_call(flag):
    try:
        prepare()
        return build(value="a" if flag else "b")
    except Error:
        return failure()


def positional(flag):
    try:
        return build("a" if flag else "b")
    except Error:
        return failure()


def trailing_argument_control(flag, value):
    try:
        return build(first=value, selected="a" if flag else "b", final=value)
    except Error:
        return failure()


def outside_try_control(flag):
    return build(value="a" if flag else "b")


async def awaited_control(client, flag):
    try:
        value = await client.load()
        return build(value="a" if flag else "b", loaded=value)
    except Error as error:
        return failure(error)


def protected_with_control(manager, flag):
    try:
        with manager:
            return build(value="a" if flag else "b")
    except Error:
        return failure()


def formatted_return(flag):
    try:
        return f"Selected: {'yes' if flag else 'no'}"
    except Error:
        return failure()


def same_line_components(flag, value):
    try:
        if value:
            return f"Value: {value}\\n" f"Attribute: {value.attribute}\\n" f"Selected: {'yes' if flag else 'no'}"
        return "missing"
    except Error:
        return failure()


def multiline_call_arguments(flag):
    try:
        return build(
            value="a" if flag else "b",
        )
    except Error:
        return failure()


def parenthesized_call_control(flag):
    try:
        return (
            build(value="a" if flag else "b")
        )
    except Error:
        return failure()


def multi_component_format_control(flag, value):
    try:
        if value:
            return (
                f"Value: {value}\\n"
                f"Attribute: {value.attribute}\\n"
                f"Selected: {'yes' if flag else 'no'}"
            )
        return "missing"
    except Error:
        return failure()
