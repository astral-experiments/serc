async def check_calls(calls, manager, prepare):
    with manager:
        await prepare()
        for call in calls:
            if call.args:
                assert not (call.args[0] == "kind" and call.args[1] == "channel")
            else:
                assert call.ready
