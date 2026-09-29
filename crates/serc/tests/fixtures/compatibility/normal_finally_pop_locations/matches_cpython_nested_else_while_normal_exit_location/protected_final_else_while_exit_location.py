def probe(local_workers, indices, in_flight, process_piece, consume_result, commit_progress):
    executor = None
    rows_seen = 0
    rows_skipped = 0
    committed_idx = None

    def submit_next():
        return schedule(executor, in_flight)

    try:
        if local_workers <= 1:
            for i in indices:
                rows_seen += 1
                if committed_idx is not None and i <= committed_idx:
                    rows_skipped += 1
                    continue
                consume_result(process_piece(i))
        else:
            executor = Pool(max_workers=local_workers)
            for _ in range(local_workers * 2):
                if not submit_next():
                    break
            while in_flight:
                future = in_flight.popleft()
                consume_result(future.result())
                submit_next()
    except KeyboardInterrupt:
        warn()
        commit_progress()
        raise
    finally:
        if executor is not None:
            executor.shutdown(wait=True, cancel_futures=True)
        commit_progress()
        if rows_skipped:
            report(rows_seen, rows_skipped)
    return iter(())
