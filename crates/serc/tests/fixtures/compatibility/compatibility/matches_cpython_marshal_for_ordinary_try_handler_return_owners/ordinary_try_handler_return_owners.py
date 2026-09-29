async def coro_two_handlers(value):
    try:
        try:
            await first(value)
        except FirstError:
            try:
                await second(value)
            except SecondError:
                observe(value)
                return value
            raise
    except OuterError:
        return None

async def coro_return_inside_new_try(value):
    try:
        await prepare(value)
        try:
            return first(value)
        except FirstError:
            try:
                return second(value)
            except SecondError:
                observe(value)
            return None
    except OuterError:
        return None

async def coro_inner_named_handler(value):
    try:
        await prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None

def sync_inner_named_handler(value):
    try:
        prepare(value)
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError as error:
                observe(error)
                return None
        return value
    except OuterError:
        return None

def sync_two_handlers(value):
    try:
        try:
            first(value)
        except FirstError:
            try:
                second(value)
            except SecondError:
                observe(value)
                return value
            raise
    except OuterError:
        return None

def sync_return_inside_new_try(value):
    try:
        prepare(value)
        try:
            return first(value)
        except FirstError:
            try:
                return second(value)
            except SecondError:
                observe(value)
            return None
    except OuterError:
        return None
