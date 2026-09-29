async def run(state, tracker):
    process: object | None = None
    server: object | None = None
    worker: object | None = None
    try:
        with manager(prefix=f"fixture-{state.name}-") as directory:
            root = make_root(directory)
            process = await launch(root)
            server = start_server(root)
            worker = start_worker(server)
            phases: list[object] = []
            initial = await process.perform(state)
            if initial.blocked and state.retry:
                tracker.note("retry")
                update = await process.perform(state)
                phases.append(update)
            summary = {
                "result": initial,
                "phases": [encode(phase) for phase in phases],
            }
            state.output = make_output(
                model=f"fixture/{state.model}",
                content=serialize(summary, ensure_ascii=False, indent=2, sort_keys=True),
            )
            return state
    except BaseException as error:
        tracker.fail(f"failed with {type(error).__name__}")
        raise
    finally:
        if process is not None and process.returncode is None:
            process.terminate()
            try:
                await wait_for(process.wait(), timeout=5)
            except TimeoutError:
                process.kill()
                await process.wait()
        if server is not None:
            await on_thread(server.shutdown)
            server.close()
        if worker is not None:
            worker.join(timeout=5)
