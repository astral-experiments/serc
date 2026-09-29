async def run():
    result = None
    try:
        try:
            await work()
        except Exception as outer_error:
            if recoverable(outer_error):
                try:
                    await recover()
                except Exception as inner_error:
                    if cancelled(inner_error):
                        raise
                    failed(inner_error)
                    result = make_result()
                    raise
            else:
                raise
        result = make_result()
        return result
    except Exception as run_error:
        failed(run_error)
        raise
    finally:
        finish()
        try:
            await publish(result)
        except Exception as publish_error:
            failed(publish_error)
            raise
