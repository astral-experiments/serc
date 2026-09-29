async def f(subject, flag, first, second):
    match subject:
        case 0:
            await first()
        case 1:
            if flag:
                await first()
            else:
                await second()
    return True
