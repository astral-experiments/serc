def unconditional_handler_return(value, flag, prior, inner_prior, outer_prior):
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return make_result(value)
        except Other as outer_exc:
            recover(outer_exc)
    finally:
        if prior is not None:
            cleanup(prior)


def boolean_handler_return(value, flag, prior, inner_prior, outer_prior):
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return make_result(value=value, error=str(exc)) or fallback
        except Other as outer_exc:
            recover(outer_exc)
    finally:
        if prior is not None:
            cleanup(prior)


async def async_handler_return(value, flag, prior, inner_prior, outer_prior):
    try:
        try:
            try:
                await action(value)
            except Error as exc:
                return make_result(value)
        except Other as outer_exc:
            recover(outer_exc)
    finally:
        if prior is not None:
            cleanup(prior)


def generator_handler_return(value, flag, prior, inner_prior, outer_prior):
    yield seed
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return make_result(value)
        except Other as outer_exc:
            recover(outer_exc)
    finally:
        if prior is not None:
            cleanup(prior)
