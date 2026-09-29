async def prepare_session(session, soft_fail=False):
    try:
        await install(session, ['base'], env={'SETUP': ''})
        await install(session, ['project'], catch=False)
        await install(session, ['checks'])
    except Exception:
        logger.exception('Setup failed', exc_info=True)
        if not soft_fail:
            raise
    finally:
        await session.container.update_network(enable_network=False)
        has_network = await container_has_network_access(session)
        if has_network:
            raise RuntimeError('Network access remains enabled')
