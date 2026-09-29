use crate::common;

const SOURCE: &str = r"def make_runner(fn, stall_multiplier, trivial_duration, max_retry, retry_reasons, stats_field):
    result_q = Queue()

    async def worker():
        updated_rows = []
        n_running_tasks = 0
        has_new_rows = False
        num_non_trivial_tasks = 0
        total_duration = 0

        async def task(row):
            nonlocal total_duration, num_non_trivial_tasks
            nonlocal result_q, has_new_rows, n_running_tasks

            handle = None
            status = {
                'status': 'pending',
                'start_time': time.time(),
                'end_time': None,
                'num_runs': 0,
                'errors': [],
                'last_traceback': None,
            }
            while True:
                if status['status'] != 'running':
                    handle = asyncio.create_task(fn(row))
                    status['status'] = 'running'
                    status['num_runs'] += 1

                try:
                    wait_time = (
                        stall_multiplier * total_duration / num_non_trivial_tasks
                        if num_non_trivial_tasks
                        else trivial_duration
                    )
                    done, pending = await asyncio.wait(
                        {handle}, timeout=wait_time, return_when=asyncio.FIRST_COMPLETED
                    )
                    if handle in pending:
                        raise asyncio.TimeoutError()
                    elif handle.exception():
                        raise handle.exception()
                    status['status'] = 'success'
                    break
                except asyncio.TimeoutError as error:
                    if num_non_trivial_tasks:
                        timeout_threshold = (
                            stall_multiplier * total_duration / num_non_trivial_tasks
                        )
                        if time.time() - status['start_time'] > timeout_threshold:
                            await cancel_and_wait(handle)
                            status['status'] = 'timeout'
                            status['timeout_threshold'] = timeout_threshold
                            break
                except Exception as error:
                    log('task failed', traceback.format_exc())
                    status['status'] = 'failed'
                    status['errors'].append(type(error).__name__)
                    status['last_traceback'] = format_error(error)
                    if type(error).__name__ not in retry_reasons:
                        break
                if status['num_runs'] > max_retry + 1:
                    break

            status['end_time'] = time.time()
            store_result(row, stats_field, status)
            if status['status'] == 'success':
                duration = status['end_time'] - status['start_time']
                if duration > trivial_duration:
                    num_non_trivial_tasks += 1
                    total_duration += duration
            result_q.put(row)
            updated_rows.append(row)
            has_new_rows = True
            n_running_tasks -= 1

        return task

    return worker
";

#[test]
fn async_retry_named_break_marker_warm_fallback_control() {
    let source = SOURCE.replace(
        "status['status'] = 'success'\n                    break",
        "status['status'] = 'success'\n                    if done:\n                        break",
    );
    common::assert_matches_cpython_marshal(&source, "async_retry_warm_fallback_control.py");
}
