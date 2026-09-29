def run_controller(controller, prepare, summarize):
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
