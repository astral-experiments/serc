def recover(work, retry, report):
    process = None
    result = ""
    try:
        process = work()
        result = process.read()
        report(result)
    except TimeoutError as error:
        result = str(error)
        if process is not None:
            token = None
            try:
                token = process.identity()
                report(token)
            except LookupError:
                token = None
            try:
                result = process.read()
                report(result)
                result = process.result if process.result is not None else result
            except TimeoutError as second_error:
                result = ""
    except OSError as error:
        result = str(error)
    report(result, process)
    return result
