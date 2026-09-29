def recover(fetch, retry, report, result, first, fallback, accepted):
    try:
        result[0] = fetch()
    except Exception as error:
        if first(error):
            report(error)
        elif fallback(error):
            try:
                result[0] = retry()
            except Exception as nested_error:
                if accepted(nested_error):
                    result[1] = True
                else:
                    raise
        else:
            raise
    return result
