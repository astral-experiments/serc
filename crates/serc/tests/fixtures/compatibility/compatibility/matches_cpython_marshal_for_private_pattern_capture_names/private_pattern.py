class Container:
    def capture(self, value):
        match value:
            case [__captured]:
                return __captured
        return None
