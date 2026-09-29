async def target(client, attempts, model, operation):
    succeeded = False
    try:
        for attempt in attempts:
            try:
                response = await client.run(attempt)
                if response is None:
                    raise Error
                sanitized = build(
                    first=select(),
                    second=select(),
                    third=select(),
                    fourth=select(),
                    extra=operation,
                )
                succeeded = True
                return sanitized
            except Error as exc:
                client.record(exc)
    finally:
        emit(
            model=model,
            operation=operation,
            response_tag="success" if succeeded else "failure",
        )
