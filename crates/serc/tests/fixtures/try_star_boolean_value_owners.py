import dis


async def named(value):
    try:
        await source
    except* Error as error:
        result = value or fallback
        consume(error, result)


async def unnamed(value):
    try:
        await source
    except* Error:
        result = value or fallback
        consume(result)


def boolean_marker_is_owned(function):
    instructions = list(dis.get_instructions(function))
    marker = next(
        current
        for previous, current in zip(instructions, instructions[1:])
        if current.opname == "NOT_TAKEN"
        and previous.opname in {"POP_JUMP_IF_FALSE", "POP_JUMP_IF_TRUE"}
    )
    return any(
        entry.start <= marker.offset < entry.end
        for entry in dis.Bytecode(function).exception_entries
    )


print(boolean_marker_is_owned(named))
print(boolean_marker_is_owned(unnamed))
