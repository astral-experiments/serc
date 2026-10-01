def runtime_list(context, values):
    with context.manager() as context:
        try:
            context.consume(values)
        finally:
            if context.active:
                context.prepare()
            try:
                context.consume(
                    ["first", "second", "third", context.category,
                     *([values] if context.option else [])]
                )
            except Failure:
                pass


def constant_list(context, values):
    with context.manager() as context:
        try:
            context.consume(values)
        finally:
            if context.active:
                context.prepare()
            try:
                context.consume(
                    ["first", "second", "third", "fourth",
                     *([values] if context.option else [])]
                )
            except Failure:
                pass


def runtime_local_list(context, values):
    with context.manager() as context:
        try:
            context.consume(values)
        finally:
            if context.active:
                context.prepare()
            try:
                context.consume(
                    ["first", "second", "third", values,
                     *([values] if context.option else [])]
                )
            except Failure:
                pass
