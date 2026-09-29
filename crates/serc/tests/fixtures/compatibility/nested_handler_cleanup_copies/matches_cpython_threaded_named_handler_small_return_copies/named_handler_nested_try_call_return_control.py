def recover(fetch, fallback, report, allow_fallback):
    try:
        return fetch()
    except Exception as error:
        if allow_fallback:
            request = error.args
            try:
                result = fallback(request)
                assert result is not None
                return result
            except Exception as nested_error:
                report(nested_error)
        else:
            report(error)
    return finish(None)
