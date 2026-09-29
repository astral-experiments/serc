def run(context, modes, left, right):
    for index, mode in enumerate(modes):
        with context.case(mode=mode):
            worker = make_worker(index)
            try:
                with (
                    helpers.patch.object(service.io, 'pipe', values=((10, 11), (12, 13))),
                    context.attach(worker),
                    helpers.patch.object(service, 'register'),
                    helpers.patch.object(service, 'publish'),
                    helpers.patch.object(service.io, 'write', result=1),
                    helpers.patch.object(service.io, 'close', callback=left),
                    helpers.patch.object(service.io, 'read', callback=right),
                    helpers.patch.object(service, 'finish') as bound,
                ):
                    if mode == 'read':
                        with context.expect(ValueError):
                            service.run(worker)
                    else:
                        result = service.run(worker)
                        context.check(result, worker)
                context.check_call(bound)
            finally:
                with service.lock:
                    service.remove(worker)
