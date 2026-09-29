async def render(self, request, payload, groups, session, network_enabled):
    try:
        for group in groups:
            for path in group:
                for attempt in range(3):
                    try:
                        success, output = await run_command(session, path)
                    except Exception as error:
                        from transport import CommandError
                        if isinstance(error, CommandError):
                            logger.error(
                                'Command failed',
                                status=error.status,
                                output=error.output.decode('utf-8', errors='replace'),
                                request_id=request.identifier,
                                attempt=attempt + 1,
                            )
                        return self.failure_output(
                            request_id=request.identifier,
                            payload=payload,
                            error_label='command failed',
                            metadata={
                                'error': 'command failed',
                                'exception': repr(error),
                                'attempt': attempt + 1,
                            },
                        )
                    if success:
                        break
        return self.success_output(request.identifier)
    finally:
        if network_enabled:
            try:
                await session.update_network(False)
                logger.info('Network disabled', request_id=request.identifier)
            except Exception:
                logger.exception('Network cleanup failed', request_id=request.identifier)
        if session is not None:
            try:
                await session.close()
            except Exception:
                logger.exception('Session cleanup failed', request_id=request.identifier)
