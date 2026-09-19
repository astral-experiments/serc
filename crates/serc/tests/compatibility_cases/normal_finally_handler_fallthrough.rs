use crate::common::assert_matches_cpython_marshal;

#[test]
fn normal_finally_nonempty_handler_fallthrough_target() {
    assert_matches_cpython_marshal(
        r"from __future__ import annotations

def process_items(factory, items, limit, now=None):
    accepted = 0
    skipped = 0
    failures = []
    acquired = False
    current_time = now or clock()
    for item in items:
        item_result: tuple[int, int, int, int] | None = None
        for attempt in range(1, limit + 1):
            resource = factory()
            attempt_failed = False
            retry_delay: float | None = None
            try:
                if not acquire(resource, item=item):
                    logger.info('busy', identify(item), describe(item))
                    skipped += 1
                    resource.rollback()
                    break
                acquired = True
                configure(resource)
                attempt_result = process(
                    resource,
                    item=item,
                    current_time=current_time,
                    limit=limit,
                )
                resource.commit()
                item_result = attempt_result
            except Exception as error:
                attempt_failed = True
                resource.rollback()
                if retryable(error) and attempt < limit:
                    retry_delay = DELAY * attempt
                    logger.warning(
                        'retry', identify(item), describe(item),
                        attempt, limit, retry_delay,
                    )
                else:
                    failures.append(Failure(item=item, error=str(error)))
                    logger.exception('failed', identify(item), describe(item), attempt)
            finally:
                resource.close()
            if retry_delay is not None:
                sleep(retry_delay)
                continue
            if attempt_failed:
                break
            break
        if item_result is None:
            continue
        first, second, third, fourth = item_result
        record(item=item, first=first, second=second, third=third, fourth=fourth)
        logger.info('complete', identify(item), describe(item), first, second, third, fourth)
        accepted += first
    result = Result(acquired=acquired, accepted=accepted, skipped=skipped, failed=len(failures))
    if failures:
        names = ', '.join(identify(failure.item) for failure in failures)
        raise BatchError(f'Failed items: {names}', result=result, failures=failures)
    return result
",
        "normal_finally_nonempty_handler_fallthrough.py",
    );
}
