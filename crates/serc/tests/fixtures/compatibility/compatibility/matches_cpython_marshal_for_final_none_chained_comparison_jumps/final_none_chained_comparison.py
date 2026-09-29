class Example:
    def __init__(self, first, second=None, prefix=None):
        if second is prefix is None:
            self.value = first
            self.first = None
            self.second = None
        else:
            self.value = prefix
            self.first = first
            self.second = second

    def render(self):
        first = self.first
        second = self.second
        if first is second is None:
            return self.value
        else:
            return f"{first}: {second}"
