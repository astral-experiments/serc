class Base:
    def method(self):
        pass


class Child(Base):
    def captured_self(self):
        def capture():
            return self
        return super().method()

    @classmethod
    def captured_cls(cls):
        def capture():
            return cls
        return super().method()

    def ordinary(self):
        return super().method()

    def explicit(self):
        def capture():
            return self
        return super(Child, self).method()
