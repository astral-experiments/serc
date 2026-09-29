def bare_return(value, prior):
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return
        except Other:
            recover()
    finally:
        cleanup(prior)


def none_return(value, prior):
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return None
        except Other:
            recover()
    finally:
        cleanup(prior)


async def number_return(value, prior):
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return 7
        except Other:
            recover()
    finally:
        cleanup(prior)


def string_generator_return(value, prior):
    yield value
    try:
        try:
            try:
                action(value)
            except Error as exc:
                return "done"
        except Other:
            recover()
    finally:
        cleanup(prior)


def popped_inner_try(value, prior):
    try:
        try:
            try:
                action()
            except Error as exc:
                try:
                    return
                except Inner:
                    recover_inner()
        except Other:
            recover()
    finally:
        cleanup()
