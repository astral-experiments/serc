async def check(cases, fetch, decode):
    failures = []
    for case in cases:
        path = case.path
        if isinstance(path, str) and path.strip():
            ok, output = await fetch(path)
            if not ok:
                failures.append(output)
            else:
                try:
                    parsed = decode(output)
                except ValueError as exc:
                    failures.append(str(exc))
                else:
                    if not isinstance(parsed, list):
                        failures.append(type(parsed).__name__)
    return failures
