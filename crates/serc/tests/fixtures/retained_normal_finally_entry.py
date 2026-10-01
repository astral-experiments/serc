async def sample(resource):
    error = None
    token = resource.token
    try:
        if token is None:
            raise RuntimeError("missing token")
        await resource.ready.wait()
        if resource.cancelled:
            await resource.worker.cancel(token)
        else:
            checkpoint = resource.checkpoint
            if checkpoint is None:
                raise RuntimeError("missing checkpoint")
            await resource.worker.commit(checkpoint)
    except BaseException as exc:
        error = exc
        logger.record("work failed", extra={"token": token, "ready": resource.checkpoint is not None})
        if token is not None:
            try:
                await resource.worker.cancel(token)
            except Exception:
                logger.record("cancel failed", extra={"token": token})
    finally:
        try:
            await resource.worker.close()
        except BaseException as exc:
            if error is None:
                error = exc
            logger.record("close failed", extra={"token": token})
        finally:
            if not resource.result.done():
                if error is None:
                    resource.result.set_result(None)
                else:
                    resource.result.set_error(error)
