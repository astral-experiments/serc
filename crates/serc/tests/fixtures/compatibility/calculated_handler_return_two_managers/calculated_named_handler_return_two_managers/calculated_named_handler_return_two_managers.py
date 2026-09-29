async def run(self, commands):
    client = normalize(self.settings.client)
    backend = 'remote' if self.settings.endpoints else 'local'
    target_kind = normalize(self.settings.target_kind)

    async def process(command: str, index: int) -> Result:
        request_id = new_request_id()
        start_time = clock()
        attempts = 0
        retry_count = 0
        events = [Event(event='start', attempt=1)]
        attributes = {
            'client': client,
            'backend': backend,
            'target_kind': target_kind,
            'operation': 'run',
        }
        metrics.increment('requests', attributes=attributes)
        with traces.enter_span('process') as span:
            span.set_attribute('request_id', request_id)
            span.set_attribute('index', index)
            last_exception = None
            with Context():
                for attempt_index in range(self.settings.retries):
                    attempts = attempt_index + 1
                    resource = None
                    try:
                        resource = acquire(self.settings.options, [Adapter(self.settings)])
                        prompt = self.settings.prompt + '\n\n' + command
                        conversation = await sample(prompt, resource)
                        message = None
                        try:
                            extracted = extract(conversation)
                            message = extracted.output
                        except Exception:
                            pass
                        duration = elapsed(start_time)
                        metrics.record('duration', duration, attributes=attributes | {'status': 'ok'})
                        span.set_attribute('status', 'ok')
                        events.append(Event(event='success', attempt=attempts, duration=duration))
                        report('success', {'request_id': request_id, 'index': index})
                        return Result(
                            message=message,
                            conversation=conversation,
                            error=None,
                            telemetry=Telemetry(
                                request_id=request_id,
                                client=client,
                                backend=backend,
                                target_kind=target_kind,
                                status='ok',
                                duration=duration,
                                attempts=attempts,
                                retry_count=retry_count,
                                error_class=None,
                                events=tuple(events),
                            ),
                        )
                    except Exception as error:
                        last_exception = error
                        error_class = classify(error)
                        if 'DEBUG' in environment:
                            print(f'Error: {error}')
                        if attempts < self.settings.retries:
                            retry_count += 1
                            metrics.increment('retries', attributes=attributes | {'reason': error_class})
                            events.append(Event(event='retry', attempt=attempts, error_class=error_class))
                            report('retry', {'request_id': request_id, **error_fields(error)})
                            await sleep(self.settings.delay)
                            continue
                        duration = elapsed(start_time)
                        metrics.increment('errors', attributes=attributes | {'error_class': error_class})
                        metrics.record('duration', duration, attributes=attributes | {'status': 'error'})
                        span.record_exception(error)
                        span.set_attribute('status', 'error')
                        span.set_attribute('error_class', error_class)
                        events.append(Event(event='error', attempt=attempts, error_class=error_class, duration=duration))
                        report('error', {'request_id': request_id, **error_fields(error)})
                        return Result(
                            message=None,
                            conversation=None,
                            error=error,
                            telemetry=Telemetry(
                                request_id=request_id,
                                client=client,
                                backend=backend,
                                target_kind=target_kind,
                                status='error',
                                duration=duration,
                                attempts=attempts,
                                retry_count=retry_count,
                                error_class=error_class,
                                events=tuple(events),
                            ),
                        )
                    finally:
                        if resource is not None:
                            await resource.aclose()
            if last_exception is None:
                last_exception = RuntimeError('No result')
            error_class = classify(last_exception)
            duration = elapsed(start_time)
            metrics.increment('errors', attributes=attributes | {'error_class': error_class})
            span.record_exception(last_exception)
            events.append(Event(event='error', attempt=attempts, error_class=error_class, duration=duration))
            report('error', {'request_id': request_id, **error_fields(last_exception)})
            return Result(
                message=None,
                conversation=None,
                error=last_exception,
                telemetry=Telemetry(
                    request_id=request_id,
                    client=client,
                    backend=backend,
                    target_kind=target_kind,
                    status='error',
                    duration=duration,
                    attempts=attempts,
                    retry_count=retry_count,
                    error_class=error_class,
                    events=tuple(events),
                ),
            )

    return await gather(
        *(process(command, index) for index, command in enumerate(commands)),
        return_exceptions=True,
    )
