def f(output, limit, timeout):
    error = None
    for attempt in range(limit):
        try:
            result = runner.run(timeout=timeout)
        except runner.Timeout:
            error = "timed out"
        else:
            if result.status == 0 and output.exists():
                try:
                    return parser.loads(output.read_text().strip())
                except parser.ParseError as exc:
                    error = f"bad output: {exc}"
            else:
                message = (result.stderr or "").strip()
                error = message or f"failed: {result.status}"
        if attempt + 1 < limit:
            delay = 1.5 * (2**attempt)
            emit(f"retry {attempt + 2}/{limit} in {delay:.1f}s")
            clock.sleep(delay)
    raise Failure(error or "failed")
