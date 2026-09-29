async def collect_attempt(config, callback):
    connection = None
    resource = None
    result = {}
    try:
        connection = await open_connection(config)
        resource = await open_resource(connection)
        await resource.__aenter__()
        result["seen"] = await wait_for_start(resource)
        if not result["seen"]:
            result["reason"] = "not_started"
            return result
        result["ready"] = await wait_for_ready(connection)
        result["complete"] = finish_state(result)
        result["completed"] = now()
        callback("complete", **result)
        return result
    except BaseException as error:
        result["error"] = format_error(error)
        result["traceback"] = traceback.format_exc(limit=8)
        result["valid"] = False
        result["reason"] = "error"
        emit("error", **result)
        return result
    finally:
        if resource is not None:
            try:
                await resource.__aexit__(None, None, None)
                result["resource_closed"] = True
            except BaseException as error:
                result["resource_closed"] = False
                result["resource_error"] = format_error(error)
        if connection is not None:
            result["close_started"] = now()
            try:
                await connection.close()
                result["close_success"] = True
            except BaseException as error:
                result["close_success"] = False
                result["close_error"] = format_error(error)
            result["close_ended"] = now()
        emit("final", **result)
