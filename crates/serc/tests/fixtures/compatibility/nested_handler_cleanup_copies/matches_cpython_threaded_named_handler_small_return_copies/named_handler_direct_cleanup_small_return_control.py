def recover(fetch, report, choose):
    result = None
    try:
        result = fetch()
    except Exception as error:
        if choose:
            report(error)
        else:
            report(error)
    return result
