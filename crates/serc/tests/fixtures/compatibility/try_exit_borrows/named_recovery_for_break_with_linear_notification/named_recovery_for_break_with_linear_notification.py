def f(executor, futures, executions):
    interruption = None
    try:
        for future in completed(futures):
            name = futures[future]
            try:
                executions[name] = result(future.result())
            except Interrupted as error:
                executions[name] = failed(error)
                interruption = error
                notify()
                for pending in futures:
                    pending.cancel()
                break
            except BaseException as error:
                executions[name] = failed(error)
    except Interrupted as error:
        interruption = error
        notify()
        for future in futures:
            future.cancel()
    finally:
        executor.shutdown(wait=True, cancel_futures=interruption is not None)
    return executions
