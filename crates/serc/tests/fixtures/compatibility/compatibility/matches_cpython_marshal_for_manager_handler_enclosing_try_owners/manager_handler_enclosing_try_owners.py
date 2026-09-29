def sync_with_inner_handler(manager, value):
    with manager:
        try:
            try:
                first(value)
            except InnerError:
                observe(value)
                return False
        except OuterError as error:
            observe(error)
            return None
        finally:
            cleanup(value)


async def coro_with_inner_handler(manager, value):
    with manager:
        try:
            try:
                await first(value)
            except InnerError:
                observe(value)
                return None
        except OuterError as error:
            observe(error)
            return None
        finally:
            cleanup(value)


async def async_with_inner_handler(manager, value):
    async with manager:
        try:
            try:
                await first(value)
            except InnerError:
                observe(value)
                return value
        except OuterError as error:
            observe(error)
            return None
        finally:
            await cleanup(value)


def without_manager(manager, value):
    try:
        try:
            first(value)
        except InnerError:
            observe(value)
            return False
    except OuterError as error:
        observe(error)
        return None
    finally:
        cleanup(value)


def without_intervening_try(manager, value):
    with manager:
        try:
            first(value)
        except InnerError:
            observe(value)
            return False
        finally:
            cleanup(value)


def sync_with_named_inner_handler(manager, value):
    with manager:
        try:
            try:
                first(value)
            except InnerError as error:
                observe(error)
                return False
        except OuterError as error:
            observe(error)
            return None
        finally:
            cleanup(value)


async def coro_with_named_inner_handler(manager, value):
    with manager:
        try:
            try:
                await first(value)
            except InnerError as error:
                observe(error)
                return None
        except OuterError as error:
            observe(error)
            return None
        finally:
            cleanup(value)


async def async_with_named_inner_handler(manager, value):
    async with manager:
        try:
            try:
                await first(value)
            except InnerError as error:
                observe(error)
                return value
        except OuterError as error:
            observe(error)
            return None
        finally:
            await cleanup(value)
