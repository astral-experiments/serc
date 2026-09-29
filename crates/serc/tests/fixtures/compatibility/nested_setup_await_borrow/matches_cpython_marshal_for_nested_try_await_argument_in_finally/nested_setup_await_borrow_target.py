from __future__ import annotations


async def collect(worker, endpoint, destination):
    details = {}
    artifact = b""
    try:
        details.update(await prepare(worker, endpoint))
        details.update(await launch(worker, endpoint))
        details.update(await wait_ready(worker, endpoint))
        details.update(await capture(worker, endpoint, destination))

        if not await exists(worker, destination):
            details["missing"] = True
            details["output"] = await read_output(worker, "run.log")
            _, listing = await execute(worker, "list", timeout=10, directory=None)
            details["listing"] = listing.strip() or None
            _, response = await execute(
                worker,
                f"probe {endpoint}",
                timeout=10,
                directory=None,
            )
            details["response"] = response.strip() or None
            details.update(await install_tools(worker))
            retry_details = await capture(worker, endpoint, destination)
            details["retry"] = retry_details
            if await exists(worker, destination):
                artifact = await worker.session.run(Download(destination))
                details["missing"] = False
        else:
            artifact = await worker.session.run(Download(destination))
    except ExecutionError as error:
        details["failed"] = True
        details["message"] = getattr(error, "message", None) or str(error)
        details["output"] = await read_output(worker, "run.log")
        artifact = b""
    finally:
        try:
            details.update(await stop(worker))
        except Exception as error:
            details["stop_error"] = f"{type(error).__name__}: {error}"
    changes = await get_changes(worker, endpoint)
    return Result(artifact=artifact, changes=changes, details=details)
