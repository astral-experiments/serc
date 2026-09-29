def recover(text, parse, retry, repair, needed, report):
    try:
        parsed = parse(text)
        recovery = {"attempted": False}
    except ValueError as error:
        attempts = []
        parsed = None
        if needed(text):
            try:
                parsed, result = retry(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as retry_error:
                attempts.append(str(retry_error))
        if parsed is None:
            try:
                parsed, result = repair(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as repair_error:
                attempts.append(str(repair_error))
                parsed, result = retry(text, repair_error)
                attempts.append(result)
                recovery = {"attempts": attempts}
    parsed["raw"] = text
    report(parsed, recovery)
    return parsed
