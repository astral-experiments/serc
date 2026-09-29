async def probe(call, collect, state, clock):
    while True:
        if state.done():
            return
        started = clock()
        try:
            await call()
        except Error as error:
            if collect:
                elapsed = clock() - started
                state.latencies.append(elapsed)
                if error.code():
                    state.timeouts += 1
                else:
                    state.errors += 1
        except TimeoutError:
            if collect:
                elapsed = clock() - started
                state.latencies.append(elapsed)
                state.timeouts += 1
        else:
            if collect:
                elapsed = clock() - started
                state.latencies.append(elapsed)
                state.successes += 1
