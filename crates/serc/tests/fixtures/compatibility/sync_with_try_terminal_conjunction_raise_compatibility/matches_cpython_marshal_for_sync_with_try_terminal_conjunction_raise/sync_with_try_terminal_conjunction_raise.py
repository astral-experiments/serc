def subprocess_shape(manager, process, value, check):
    with manager:
        try:
            result = process.communicate(value)
        except Timeout as exc:
            process.kill()
            process.wait()
            raise
        except BaseException:
            process.kill()
            raise
        retcode = process.poll()
        if check and retcode:
            raise CalledProcessError(retcode, result)
    return result

def single_handler(manager, first, second, work, cleanup):
    with manager:
        try:
            work()
        except Error:
            cleanup()
            raise
        if first and second:
            raise OtherError

def fallthrough_handler(manager, first, second, work, cleanup):
    with manager:
        try:
            work()
        except Error:
            cleanup()
        if first and second:
            raise OtherError

def finally_shape(manager, first, second, work, cleanup):
    with manager:
        try:
            work()
        finally:
            cleanup()
        if first and second:
            raise OtherError

def three_terms(manager, first, second, third, work, cleanup):
    with manager:
        try:
            work()
        except Error:
            cleanup()
            raise
        if first and second and third:
            raise OtherError
