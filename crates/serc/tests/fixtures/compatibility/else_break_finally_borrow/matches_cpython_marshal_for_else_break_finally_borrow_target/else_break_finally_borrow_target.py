async def run(state):
    while True:
        delay = state.next_delay()
        if delay <= 0:
            break
        try:
            await scheduler.wait_condition(
                state.interrupted,
                timeout=delay,
                timeout_summary="Waiting before next check",
            )
        except TimeoutError:
            pass
        else:
            if state.done():
                break
        finally:
            state.wake.clear()
        if state.stop() or state.skip.is_set():
            break
    return None
