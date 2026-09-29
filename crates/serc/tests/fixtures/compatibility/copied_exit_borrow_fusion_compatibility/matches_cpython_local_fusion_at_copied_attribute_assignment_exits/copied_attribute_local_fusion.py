class Config:
    def __init__(self, value):
        self.value = value if value is not None else []

    def replace(self, flag, value):
        self.value = value if flag else None

    def replace_and_return(self, flag, value):
        self.value = value if flag else None
        return value
