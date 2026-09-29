async def record_history(read, history, store, enabled, limit):
    while True:
        answer = read()
        if answer is None:
            break
        try:
            if answer:
                compact = answer if len(answer) <= limit else answer[:limit]
                history.append(f"Q: {answer}")
                history.append(f"A: {compact}")
                try:
                    if enabled and store is not None:
                        store.append_string(answer)
                except Exception:
                    pass
                history.append("done")
        except Exception:
            pass
