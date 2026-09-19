use crate::common;

#[test]
fn finally_for_borrow_async_break_target() {
    let source = r"async def run_jobs(config, tasks, cancelled_error, sleep, gather, report):
    reason = 'running'
    try:
        if config.invalid:
            raise ValueError()
        while True:
            stopping = config.stopping
            done = {task for task in tasks if task.done()}
            for task in done:
                tasks.remove(task)
                report(task.result())
            if stopping and not tasks:
                reason = 'expired' if config.expired else 'limit'
                break
            while config.should_launch():
                tasks.add(config.start_task())
            if config.idle:
                await sleep()
    except cancelled_error:
        reason = 'cancelled'
        raise
    except Exception:
        reason = 'error'
        raise
    finally:
        for task in tasks:
            task.cancel()
        if tasks:
            await gather(*tasks, return_exceptions=True)
        report(reason)
    return reason
";
    common::assert_matches_cpython_marshal(source, "finally_for_borrow_async_break_target.py");
}

#[test]
fn finally_for_borrow_handler_break_target() {
    let source = r"def run_controller(controller, prepare, summarize):
    try:
        prepare(controller.configuration)
        while True:
            item = controller.incoming.get()
            if item is controller.stop:
                break
            slot, rows = item
            staged = controller.stage(int(slot), list(rows))
            averages = summarize(staged)
            for row in staged:
                controller.tasks.put((row, averages.get(row['key'])))
    except BaseException as error:
        controller.error = error
        controller.failed.set()
        controller.update_status(reason=f'{type(error).__name__}: {error}')
    finally:
        for worker in controller.workers:
            controller.tasks.put(controller.stop)
";
    common::assert_matches_cpython_marshal(source, "finally_for_borrow_handler_break_target.py");
}

#[test]
fn finally_for_borrow_conditional_target() {
    let source = r"def run_workers(count, process_factory, client_mode, create_loop, submit, sample, queue, stop):
    workers = []
    try:
        for index in range(count):
            process = process_factory(index, queue, stop)
            process.start()
            workers.append(process)
        if client_mode:
            with create_loop() as loop:
                future = submit(sample(workers), loop)
                future.result()
        else:
            sample(workers)
    except KeyboardInterrupt:
        print('Interrupted')
        stop.set()
        for process in workers:
            process.terminate()
    finally:
        for process in workers:
            process.join()
        queue.close()
        queue.join_thread()
    return 0
";
    common::assert_matches_cpython_marshal(source, "finally_for_borrow_conditional_target.py");
}

#[test]
fn finally_for_borrow_call_entry_control() {
    let source = r"def run_controller(controller, prepare, summarize, cleanup_started):
    try:
        prepare(controller.configuration)
        while True:
            item = controller.incoming.get()
            if item is controller.stop:
                break
            slot, rows = item
            staged = controller.stage(int(slot), list(rows))
            averages = summarize(staged)
            for row in staged:
                controller.tasks.put((row, averages.get(row['key'])))
    except BaseException as error:
        controller.error = error
        controller.failed.set()
        controller.update_status(reason=f'{type(error).__name__}: {error}')
    finally:
        cleanup_started()
        for worker in controller.workers:
            controller.tasks.put(controller.stop)
";
    common::assert_matches_cpython_marshal(source, "finally_for_borrow_call_entry_control.py");
}
