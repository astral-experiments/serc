async def choose(self, mode):
    try:
        match self.kind:
            case Choices.first:
                await self.first(mode)
            case Choices.second:
                await self.second(mode)
    except Error as error:
        raise OtherError() from error
    finally:
        await self.coordinator.refresh()
