def generator_final_elif_raise(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if not stack:
                raise RuntimeError(value)
            stack.pop()


def generator_final_elif_name_return(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if stack:
                return value
            stack.append(value)


def generator_final_elif_nested_else(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            if not stack:
                stack.append(value)
            else:
                stack.pop()


def generator_multiple_elif_nested_continue(values, stack):
    for value in values:
        if value.first or value.second:
            yield value
        elif value.third:
            yield from prepare(value)
        elif value.fourth:
            if not stack:
                continue
            stack.pop()


async def coroutine_final_elif_nested_break(values, stack):
    for value in values:
        if value.first or value.second:
            await observe(value)
        elif value.third:
            if not stack:
                break
            stack.pop()
