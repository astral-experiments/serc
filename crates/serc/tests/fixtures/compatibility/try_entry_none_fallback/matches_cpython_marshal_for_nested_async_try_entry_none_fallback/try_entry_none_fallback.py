async def process(pending, queue, patches, worker):
    attempted = succeeded = failed = 0

    async def flush():
        nonlocal attempted, succeeded, failed
        if not queue:
            return

        results = []
        if pending:
            attempted += len(pending)
            results = await worker(pending)

        by_id = {}
        for index, item in enumerate(pending):
            by_id[id(item)] = results[index]

        for obj in queue:
            meta = metadata(obj)
            wrote = False
            raw = ""
            result = by_id.get(id(obj), None)
            reuse = patches.get(id(obj), None)
            try:
                if result is not None:
                    raw, parsed, error = result
                    if error is not None or parsed is None:
                        raise ValueError(error or "invalid result")
                    value, basis, evidence = validate(parsed)
                elif reuse is not None:
                    value = reuse["value"]
                    basis = reuse.get("basis", None) or "existing"
                    evidence = reuse.get("evidence", None) or []
                    raw = ""
                else:
                    raise ValueError("missing result")
                meta["value"] = value
                meta["basis"] = basis
                meta["evidence"] = evidence
                wrote = True
                succeeded += 1
            except Exception as error:
                meta["error"] = str(error)
                wrote = False
                failed += 1
            emit(meta, raw, wrote)

    await flush()
