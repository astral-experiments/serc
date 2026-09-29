def f(data, check, load, report):
    result = load(data)
    try:
        report(result)
        if data and check(data):
            assert result.ready, "not ready"
            try:
                assert result.value == {} or result.value is None
                result.value = {}
            except Exception:
                pass
    except (AssertionError, ValueError, TypeError):
        report(data, result)
        return []
    answers = []
    for entry in data.get("entries", []):
        answers.append((entry, load(entry), result))
    report(data, answers)
    if check(answers):
        answers.append(result)
    return answers
