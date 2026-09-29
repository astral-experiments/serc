async def f(convo, delegate):
    i = 0
    message = convo.messages[i]
    if message.role == 0:
        i += 1
    elif message.role == 1:
        i += 1
        if delegate:
            first = True
            while i < len(convo.messages):
                if convo.messages[i].role != 1:
                    i += 1
                else:
                    break
        else:
            other()
