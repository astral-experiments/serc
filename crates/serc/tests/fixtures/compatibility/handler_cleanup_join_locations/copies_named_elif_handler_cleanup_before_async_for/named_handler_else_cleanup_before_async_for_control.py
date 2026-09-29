class Wrapper(Base):
    async def process(self, message):
        if self.enabled:
            text = render(message)
            try:
                result = loads(text)
            except DecodeError as error:
                text = repair(text)
                if isinstance(message.content, Text):
                    message.content.parts[0] = text
                else:
                    message.content.text = text
            except Exception as other:
                pass
        async for message in super().process(message):
            yield message
