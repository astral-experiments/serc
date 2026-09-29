def named_handler_trailing_with(outer, inner):
    with outer:
        try:
            action()
        except Error as error:
            with inner:
                recover(error)
        else:
            finish()
