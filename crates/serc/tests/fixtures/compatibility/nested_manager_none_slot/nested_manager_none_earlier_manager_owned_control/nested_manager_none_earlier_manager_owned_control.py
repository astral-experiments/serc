def run(context, flag):
    with (
        context.manager(1),
        context.manager(2),
        context.manager(3),
        context.manager(4),
        context.manager(5) as stream,
    ):
        value = stream.read() if flag is None else stream
        consume(value)
