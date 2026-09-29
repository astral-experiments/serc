def worker(datapoint, config, result_path, limit):
    result = {"status": "failed"}
    try:
        manager = nullcontext()
        if limit is not None:
            manager = runtime_context(limit)
        with manager:
            with source_context(config):
                record = datapoint.to_record(config=config)
        payload = serialize(record)
        result = {"status": "completed", "payload": payload}
    except BaseException as error:
        current_error = error
        requires_runtime = False
        while current_error is not None:
            if isinstance(current_error, RuntimeNeeded):
                requires_runtime = True
                break
            current_error = current_error.__cause__ or current_error.__context__
        result = {
            "status": "failed",
            "error_type": type(error).__name__,
            "error": str(error),
            "traceback": traceback.format_exc(),
            "requires_runtime": requires_runtime,
        }
    finally:
        try:
            with open(result_path, "wb") as result_file:
                pickle.dump(result, result_file, protocol=pickle.HIGHEST_PROTOCOL)
        finally:
            release_memory()
