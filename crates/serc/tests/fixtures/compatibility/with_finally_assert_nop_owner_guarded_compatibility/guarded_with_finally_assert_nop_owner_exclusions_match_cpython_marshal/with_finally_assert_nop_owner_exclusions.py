def no_with(condition):
    try:
        assert condition
    finally:
        cleanup()


def non_assert_tail(manager, condition):
    with manager:
        try:
            consume(condition)
        finally:
            cleanup()


def handler_form(manager, condition):
    with manager:
        try:
            assert condition
        except Error:
            pass
        finally:
            cleanup()
        after()


def enclosing_try(manager, condition):
    with manager:
        try:
            try:
                assert condition
            finally:
                cleanup()
        except Error:
            pass


def enclosing_handler(manager, condition):
    with manager:
        try:
            body()
        except Error as error:
            try:
                assert condition
            finally:
                consume(error)


def enclosing_finally(manager, condition):
    with manager:
        try:
            raise Error()
        finally:
            try:
                assert condition
            finally:
                cleanup()
