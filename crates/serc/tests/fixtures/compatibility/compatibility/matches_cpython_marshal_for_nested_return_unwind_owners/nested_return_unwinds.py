def loop_value(items):
    try:
        for item in items:
            try:
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                use(item)
                return item
            except Error:
                pass
    finally:
        cleanup()
    return None


def literal_values(manager):
    try:
        with manager:
            prepare()
        try:
            action()
            return True
        except Error:
            return False
    finally:
        cleanup()


def nested_handler(compiler):
    try:
        command = compiler.compiler
    except AttributeError:
        try:
            if not compiler.initialized:
                compiler.initialize()
        except (SetupError, ValueError):
            return False
        command = [compiler.cc]
    return command
