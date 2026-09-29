class Container:
    def nested_handler(self):
        try:
            body()
        except ValueError as __error:
            try:
                pass
            except TypeError:
                raise __error
