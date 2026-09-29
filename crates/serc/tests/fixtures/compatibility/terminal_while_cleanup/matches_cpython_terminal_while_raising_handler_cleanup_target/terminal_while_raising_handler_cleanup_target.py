
def retry(request):
    result = None
    resource = None
    command = "worker " + request.name
    try:
        resource = start(command, isolated=True, shell=True)
        attempts = 10
        while attempts > 0:
            try:
                result = client.complete(
                    name="dummy",
                    key="dummy",
                    endpoint="local",
                    prompt=request.prompt,
                    limit=10,
                    scores=0,
                    echo=True,
                    temperature=0.0,
                )
            except Exception as error:
                report(f"Retry pending: {error}")
                pause(10)
                attempts -= 1
            else:
                break
    except Exception as error:
        report("Worker failed.")
        raise error
    finally:
        if resource is not None:
            process.stop(process.group(resource.pid), signal.STOP)
    if result is None:
        raise RuntimeError("No result")
    return result
