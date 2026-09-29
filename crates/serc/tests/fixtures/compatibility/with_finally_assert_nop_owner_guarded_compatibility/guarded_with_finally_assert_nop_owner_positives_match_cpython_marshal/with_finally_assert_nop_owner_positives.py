def spack_followed_cleanup(manager, repo, old_path, pkg):
    with manager:
        try:
            repo.python_path = None
            assert github_url(pkg) is None
            repo.python_path = "/missing"
            assert github_url(pkg) is None
        finally:
            repo.python_path = old_path
        after()


async def coroutine(manager, condition):
    with manager:
        try:
            assert await check(condition)
        finally:
            cleanup()


async def asynchronous_with(manager, condition):
    async with manager:
        try:
            assert condition
        finally:
            await cleanup()


def generator(manager, condition):
    yield seed()
    with manager:
        try:
            assert condition
        finally:
            cleanup()


async def async_generator(manager, condition):
    yield seed()
    with manager:
        try:
            assert condition
        finally:
            await cleanup()
