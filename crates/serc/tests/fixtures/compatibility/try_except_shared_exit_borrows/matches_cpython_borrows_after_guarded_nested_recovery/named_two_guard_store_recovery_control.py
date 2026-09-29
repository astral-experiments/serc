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
                recovery = None
        if parsed is None:
            try:
                parsed, result = repair(text, error)
                attempts.append(result)
                recovery = {"attempts": attempts}
            except Exception as repair_error:
                recovery = None
    parsed["raw"] = text
    report(parsed, recovery)
    return parsed
