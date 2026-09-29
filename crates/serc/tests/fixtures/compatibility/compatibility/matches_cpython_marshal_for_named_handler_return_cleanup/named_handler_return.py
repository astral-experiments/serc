class Container:
    def return_from_handler(self):
        try:
            body()
        except Error as __error:
            return __error
