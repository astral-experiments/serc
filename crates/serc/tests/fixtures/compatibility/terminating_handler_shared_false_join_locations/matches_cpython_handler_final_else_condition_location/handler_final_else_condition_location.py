def retry(attempts, send, log, special):
    for attempt in range(attempts):
        try:
            response = send()
            log(response)
        except LookupError as error:
            if special(error):
                log(error)
                return None
            else:
                log(error)
                if attempt == attempts - 1:
                    return None
        except Exception as error:
            log(error)
            if attempt == attempts - 1:
                return None
        else:
            assert isinstance(response, dict)
            return response
    raise AssertionError
