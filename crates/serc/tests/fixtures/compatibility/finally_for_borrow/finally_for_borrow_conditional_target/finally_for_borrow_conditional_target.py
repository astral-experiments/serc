def run_workers(count, process_factory, client_mode, create_loop, submit, sample, queue, stop):
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
