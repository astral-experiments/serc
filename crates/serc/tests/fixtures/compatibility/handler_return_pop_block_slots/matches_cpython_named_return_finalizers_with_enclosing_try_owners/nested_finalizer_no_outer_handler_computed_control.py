def build_request(worker, manager, active, missing, timeout):
    async def run():
        token = None
        if active:
            token = worker.context.set("request")
        try:
            if missing:
                return response(status=401, data={"error": "missing"})
            try:
                with manager():
                    await worker.validate()
            except LookupError as error:
                return response(status=error.status, data={"error": str(error)})
            with manager():
                if timeout is None:
                    result = await worker.fetch()
                else:
                    result = await worker.wait(timeout=float(timeout))
        finally:
            if token is not None:
                worker.session.remove()
                worker.replica().remove()
                worker.context.reset(token)
        return response(status=200, data=result)
    return run
