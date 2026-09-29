class VisualCompiler:
    if platform:
        try:
            compiler = make_compiler()
            compiler.initialize()
        except AttributeError as error:
            if "root" in str(error):
                report(error)
            else:
                raise
        except OSError as error:
            if not expected(error):
                report(error)
                raise

    executable = compiler


def terminating_body_control(flag):
    try:
        action()
    except Error as error:
        if flag:
            raise
        else:
            recover(error)
    after()


def outside_handler_control(flag):
    if flag:
        recover()
    else:
        raise Error
    after()
