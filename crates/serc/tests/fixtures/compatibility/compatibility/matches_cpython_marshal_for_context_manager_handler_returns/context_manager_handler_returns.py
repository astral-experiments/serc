def sync_direct(manager, error):
    with manager:
        try:
            pass
        except error:
            return


def sync_conditional(manager, error, flag):
    with manager:
        try:
            pass
        except error:
            if flag:
                return


def sync_conditional_tail(manager, error, flag, cleanup):
    with manager:
        try:
            cleanup()
        except error:
            if flag:
                return True
            cleanup()


def sync_else(manager, error, action, cleanup):
    with manager:
        try:
            action()
        except error:
            return True
        else:
            cleanup()


def sync_nested(outer, inner, error):
    with outer:
        with inner:
            try:
                pass
            except error:
                return


async def async_direct(manager, error):
    async with manager:
        try:
            pass
        except error:
            return


async def async_literal(manager, error):
    async with manager:
        try:
            pass
        except error:
            return True


async def async_action(manager, error, action):
    async with manager:
        try:
            action()
        except error:
            return


async def async_conditional(manager, error, flag, action):
    async with manager:
        try:
            action()
        except error:
            if flag:
                return


async def async_two_handlers(manager, first, second):
    async with manager:
        try:
            pass
        except first:
            return
        except second:
            pass


async def async_nested(outer, inner, error):
    async with outer:
        async with inner:
            try:
                pass
            except error:
                return


async def sync_then_async(outer, inner, error):
    with outer:
        async with inner:
            try:
                pass
            except error:
                return


async def async_then_sync(outer, inner, error):
    async with outer:
        with inner:
            try:
                pass
            except error:
                return


async def async_while(manager, error, flag):
    async with manager:
        while flag:
            try:
                pass
            except error:
                return


async def async_generator(manager, error, value):
    async with manager:
        try:
            yield value
        except error:
            return


def sync_raise_control(manager, error):
    with manager:
        try:
            pass
        except error:
            raise


async def async_raise_control(manager, error):
    async with manager:
        try:
            pass
        except error:
            raise
